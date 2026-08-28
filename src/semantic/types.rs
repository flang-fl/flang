#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Integer(IntegerType),
    Bool,
    Unit,
    FixedArray {
        size: usize,
        base_type: Box<Type>,
    },
    Function {
        parameters: Vec<Type>,
        return_type: Box<Type>,
    },
    Error,
    Unknown,
    Type,
}

impl Type {
    pub fn as_integer(&self) -> Option<IntegerType> {
        match self {
            Self::Integer(integer) => Some(*integer),
            _ => None,
        }
    }

    pub fn is_integer(&self) -> bool {
        self.as_integer().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerType {
    I32,
    I64,

    U32,
    Usize
}

impl IntegerType {
    pub fn is_signed(self) -> bool {
        matches!(self, Self::I32 | Self::I64)
    }

    pub fn bit_width(self) -> u32 {
        match self {
            Self::I32 | Self::U32 => 32,
            Self::I64 => 64,
            Self::Usize => usize::BITS
        }
    }

    pub fn maximum_literal(self) -> u64 {
        match self {
            Self::I32 => i32::MAX as u64,
            Self::I64 => i64::MAX as u64,
            Self::U32 => u32::MAX as u64,
            Self::Usize => usize::MAX as u64,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U32 => "u32",
            Self::Usize => "usize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionTypeId(usize);

#[derive(Debug, Clone)]
pub struct FunctionType {
    return_type: Type,
}