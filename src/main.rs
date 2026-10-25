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
    let status =
        match latex_lsp::server::protocol::serve(tokio::io::stdin(), tokio::io::stdout()).await {
            Ok(status) => status,
            Err(error) => {
                eprintln!("latex-lsp: {error}");
                1
            }
        };
    // Tokio's stdin may retain a blocked reader when the editor sends exit
    // without closing the pipe. Cleanup has completed; do not wait for EOF.
    std::process::exit(status);
}
