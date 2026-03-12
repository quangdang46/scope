pub mod parser;

pub fn greet(name: &str) -> String {
    parser::parse(name).join(" ")
}
