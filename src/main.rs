use latex_lsp::Backend;
use std::task::{Context, Poll};
use tower::Service;
use tower_lsp::jsonrpc::Request;
use tower_lsp::{LspService, Server};

/// tower-lsp closes its client socket on exit, but its stdio reader still waits
/// for EOF. Terminate the standalone process when the client sends exit, even
/// if the client keeps its stdin pipe open. LSP requires status 1 without shutdown.
struct Lifecycle(LspService<Backend>);

impl Service<Request> for Lifecycle {
    type Response = <LspService<Backend> as Service<Request>>::Response;
    type Error = <LspService<Backend> as Service<Request>>::Error;
    type Future = <LspService<Backend> as Service<Request>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        if request.method() == "exit" && request.id().is_none() {
            std::process::exit(if self.0.inner().is_shutdown() { 0 } else { 1 });
        }
        self.0.call(request)
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
    let (service, socket) = LspService::new(Backend::new);
    // Preserve wire order for changes and requests; analysis uses in-memory snapshots.
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .concurrency_level(1)
        .serve(Lifecycle(service))
        .await;
}
