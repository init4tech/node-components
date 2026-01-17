macro_rules! table {
    (
        @implement
        #[doc = $doc:expr]
        $name:ident, $key:ty, $value:ty, $dual:expr, $fixed:expr
    ) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::tables::Table for $name {
            const NAME: &'static str = stringify!($name);
            const DUAL_KEY: bool = $dual;
            const FIXED_VAL_SIZE: Option<usize> = $fixed;

            type Key = $key;
            type Value = $value;
        }

    };

    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $value:ty>
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            false,
            None
        );

        impl crate::tables::SingleKey for $name {}
    };


    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty>
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            true,
            None
        );

        impl crate::tables::DualKeyed for $name {
            type Key2 = $subkey;
        }
    };

    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> is $fixed:expr
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            true,
            Some($fixed)
        );

        impl crate::tables::DualKeyed for $name {
            type Key2 = $subkey;
        }
    };
}
