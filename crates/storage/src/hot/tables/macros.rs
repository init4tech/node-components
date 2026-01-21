macro_rules! table {
    (
        @implement
        #[doc = $doc:expr]
        $name:ident, $key:ty, $value:ty, $dual:expr, $fixed:expr
    ) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::hot::tables::Table for $name {
            const NAME: &'static str = stringify!($name);
            const FIXED_VAL_SIZE: Option<usize> = $fixed;
            const DUAL_KEY_SIZE: Option<usize> = $dual;
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
            None,
            None
        );

        impl crate::hot::tables::SingleKey for $name {}
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
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            None
        );

        impl crate::hot::tables::DualKey for $name {
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
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            Some($fixed)
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
        }
    };
}
