macro_rules! tables {
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $value:ty>
    ) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::tables::Table for $name {
            const NAME: &'static str = stringify!($name);

            type Key = $key;
            type Value = $value;
        }
    };

    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> size: $fixed:expr
    ) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::tables::DualKeyed for $name {
            const NAME: &'static str = stringify!($name);

            const FIXED_VALUE_SIZE: Option<usize> = { $fixed };

            type K1 = $key;
            type K2 = $subkey;

            type Value = $value;
        }
    };

    ($(#[doc = $doc:expr] $name:ident<$key:ty => $value:ty>),* $(,)?) => {
        $(
            tables!(#[doc = $doc] $name<$key => $value>);
        )*
    };

    ($(#[doc = $doc:expr] $name:ident<$key:ty => $subkey:ty => $value:ty> size: $fixed:expr),* $(,)?) => {
        $(
            tables!(#[doc = $doc] $name<$key => $subkey => $value> size: $fixed);
        )*
    };
}
