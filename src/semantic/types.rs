use crate::comptime::FunctionTemplateId;
use crate::semantic::symbols::ModuleId;
use crate::TargetInfo;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Integer(IntegerType),
    Module(ModuleId),
    Bool,
    Str,
    Unit,
    FixedArray {
        size: usize,
        base_type: Box<Type>,
    },
    Function {
        parameters: Vec<Type>,
        return_type: Box<Type>,
    },
    FunctionTemplate(FunctionTemplateId),
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ComptimeKey {
    Integer {
        value: i128,
        type_: IntegerType
    },
    Str(String),
    Type(Type),
    Bool(bool)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpecializationKey {
    pub template: FunctionTemplateId,
    pub arguments: Vec<ComptimeKey>
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    pub fn minimum_value(self, target: &TargetInfo) -> i128 {
        use IntegerType::*;

        match self {
            U8 | U16 | U32 | U64 | Usize => 0,

            I8 => i8::MIN as i128,
            I16 => i16::MIN as i128,
            I32 => i32::MIN as i128,
            I64 => i64::MIN as i128,

            Isize => match target.pointer_bit_width {
                32 => i32::MIN as i128,
                64 => i64::MIN as i128,

                width => {
                    panic!("unsupported pointer width: {width}")
                }
            },
        }
    }

    pub fn maximum_value(self, target: &TargetInfo) -> i128 {
        use IntegerType::*;
        match self {
            U8 => u8::MAX as i128,
            I8 => i8::MAX as i128,
            U16 => u16::MAX as i128,
            I16 => i16::MAX as i128,
            U32 => u32::MAX as i128,
            I32 => i32::MAX as i128,
            U64 => u64::MAX as i128,
            I64 => i64::MAX as i128,
            Isize => match target.pointer_bit_width {
                32 => i32::MAX as i128,
                64 => i64::MAX as i128,
                width => {
                    panic!("unsupported pointer width: {width}")
                }
            },

            Usize => match target.pointer_bit_width {
                32 => u32::MAX as i128,
                64 => u64::MAX as i128,
                width => {
                    panic!("unsupported pointer width: {width}")
                }
            },
        }
    }

    pub fn contains(self, value: i128, target: &TargetInfo) -> bool {
        value >= self.minimum_value(target) && value <= self.maximum_value(target)
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
