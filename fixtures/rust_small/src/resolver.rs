use crate::parser;

pub fn resolve(input: &str) -> String {
    parser::parse(input).join("::")
}
