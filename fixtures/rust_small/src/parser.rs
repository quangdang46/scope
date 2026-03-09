pub fn parse(input: &str) -> Vec<String> {
    tokenize(input)
}

fn tokenize(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}
