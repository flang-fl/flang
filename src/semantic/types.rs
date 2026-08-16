#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    I64,
    Bool,
    Unit,
    Function {
        parameters: Vec<Type>,
        return_type: Box<Type>,
    },
    Error,
    Unknown,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionTypeId(usize);

#[derive(Debug, Clone)]
pub struct FunctionType {
    return_type: Type,
}