use crate::TargetInfo;

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
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,

    Usize,
    Isize,
}

impl IntegerType {
    pub fn is_signed(self) -> bool {
        use IntegerType::*;
        matches!(self, I8 | I16 | I32 | I64 | Isize)
    }

    pub fn bit_width(self, target: &TargetInfo) -> u32 {
        use IntegerType::*;
        match self {
            U8 | I8 => 8,
            U16 | I16 => 16,
            I32 | U32 => 32,
            U64 | I64 => 64,
            Usize | Isize => target.pointer_bit_width,
        }
    }

    pub fn maximum_literal(self, target: &TargetInfo) -> u64 {
        use IntegerType::*;
        match self {
            U8 => u8::MAX as u64,
            I8 => i8::MAX as u64,
            U16 => u16::MAX as u64,
            I16 => i16::MAX as u64,
            U32 => u32::MAX as u64,
            I32 => i32::MAX as u64,
            U64 => u64::MAX,
            I64 => i64::MAX as u64,
            Isize => match target.pointer_bit_width {
                32 => i32::MAX as u64,
                64 => i64::MAX as u64,
                width => {
                    panic!("unsupported pointer width: {width}")
                }
            },

            Usize => match target.pointer_bit_width {
                32 => u32::MAX as u64,
                64 => u64::MAX,
                width => {
                    panic!("unsupported pointer width: {width}")
                }
            },
        }
    }

    pub fn name(self) -> &'static str {
        use IntegerType::*;
        match self {
            U8 => "u8",
            I8 => "i8",
            U16 => "u16",
            I16 => "i16",
            U32 => "u32",
            I32 => "i32",
            U64 => "u64",
            I64 => "i64",
            Usize => "usize",
            Isize => "isize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionTypeId(usize);

#[derive(Debug, Clone)]
pub struct FunctionType {
    return_type: Type,
}
