fn main() {
    let output = serde_json::to_string_pretty(&scope_core::stub::mcp_stub_message())
        .expect("scope-mcp stub should serialize");

    println!("{output}");
}
