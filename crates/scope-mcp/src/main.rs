use std::io::{self, Write};

fn main() {
    if let Err(error) = scope_mcp::run() {
        let _ = writeln!(io::stderr(), "scope-mcp fatal error: {error}");
        std::process::exit(1);
    }
}
