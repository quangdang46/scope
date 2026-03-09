pub mod parser;
pub mod resolver;
pub mod utils;

pub fn greet(name: &str) -> String {
    let tokens = parser::parse(name);
    utils::format_output(&tokens.join(" "))
}

pub fn farewell(name: &str) -> String {
    format!("goodbye, {name}")
}
