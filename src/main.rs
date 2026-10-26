#[tokio::main]
async fn main() {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("texquill {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("texquill {}\n\nLaTeX language server over stdio.\n\nUsage: texquill [--stdio]\n       texquill --version", env!("CARGO_PKG_VERSION"));
        return;
    }
    let status =
        match texquill::server::protocol::serve(tokio::io::stdin(), tokio::io::stdout()).await {
            Ok(status) => status,
            Err(error) => {
                eprintln!("texquill: {error}");
                1
            }
        };
    // Tokio's stdin may retain a blocked reader when the editor sends exit
    // without closing the pipe. Cleanup has completed; do not wait for EOF.
    std::process::exit(status);
}
