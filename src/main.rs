use latex_lsp::Backend;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::oneshot,
};
use tower::Service;
use tower_lsp::jsonrpc::Request;
use tower_lsp::{LspService, Server};

/// tower-lsp closes its client socket on exit, but its stdio reader still waits
/// for EOF. Terminate the standalone process when the client sends exit, even
/// if the client keeps its stdin pipe open. LSP requires status 1 without shutdown.
struct Lifecycle {
    service: LspService<Backend>,
    queue: latex_lsp::dispatch::RequestQueue,
    exit_code: Arc<AtomicI32>,
}

impl Service<Request> for Lifecycle {
    type Response = <LspService<Backend> as Service<Request>>::Response;
    type Error = <LspService<Backend> as Service<Request>>::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        if request.method() == "exit" && request.id().is_none() {
            let status = if self.service.inner().is_shutdown() {
                0
            } else {
                1
            };
            let cleanup = self.service.inner().disconnect();
            self.exit_code.store(status, Ordering::SeqCst);
            return Box::pin(async move {
                cleanup.await;
                std::process::exit(status);
            });
        }
        let future = self.service.call(request);
        Box::pin(self.queue.ordered(future))
    }
}

/// Start cleanup as soon as the transport closes. Waiting until `serve`
/// returns can leave it draining an in-flight request for an external process.
struct Input<R> {
    reader: R,
    closed: Option<oneshot::Sender<()>>,
}

impl<R: AsyncRead + Unpin> AsyncRead for Input<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let available = buffer.remaining();
        let result = Pin::new(&mut self.reader).poll_read(cx, buffer);
        let closed = matches!(&result, Poll::Ready(Err(_)))
            || (matches!(&result, Poll::Ready(Ok(())))
                && available > 0
                && buffer.filled().len() == before);
        if closed {
            if let Some(sender) = self.closed.take() {
                let _ = sender.send(());
            }
        }
        result
    }
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("latex-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("latex-lsp {}\n\nLaTeX language server over stdio.\n\nUsage: latex-lsp [--stdio]\n       latex-lsp --version", env!("CARGO_PKG_VERSION"));
        return;
    }
    let (service, socket) = LspService::build(Backend::new)
        .custom_method("window/workDoneProgress/cancel", Backend::cancel_progress)
        .finish();
    let cleanup = service.inner().disconnect();
    let (closed, disconnected) = oneshot::channel();
    let mut cleanup_task = tokio::spawn(async move {
        // Dropping the reader after any transport failure also closes the channel.
        let _ = disconnected.await;
        cleanup.await;
    });
    let input = Input {
        reader: tokio::io::stdin(),
        closed: Some(closed),
    };
    // Ordinary handlers retain wire order. Snapshot-based external queries can
    // release their turn while waiting for subprocess output.
    let exit_code = Arc::new(AtomicI32::new(0));
    let serve = Server::new(input, tokio::io::stdout(), socket)
        .concurrency_level(32)
        .serve(Lifecycle {
            service,
            queue: Default::default(),
            exit_code: exit_code.clone(),
        });
    tokio::pin!(serve);
    tokio::select! {
        _ = &mut serve => { let _ = cleanup_task.await; }
        // A client request (for example watcher registration) may never be
        // acknowledged after EOF. Once external work is reaped, drop the
        // remaining RPC handlers instead of waiting for those responses.
        _ = &mut cleanup_task => {}
    }
    let status = exit_code.load(Ordering::SeqCst);
    if status != 0 {
        std::process::exit(status);
    }
}
