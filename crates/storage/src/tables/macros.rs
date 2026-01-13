macro_rules! tables {
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty, $value:ty>) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::tables::Table for $name {
            const NAME: &'static str = stringify!($name);
            type Key = $key;
            type Value = $value;
        }
    };

    ($(#[doc = $doc:expr] $name:ident<$key:ty, $value:ty>),* $(,)?) => {
        $(
            tables!(#[doc = $doc] $name<$key, $value>);
        )*
    };
}
