macro_rules! table {
    (
        @implement
        #[doc = $doc:expr]
        $name:ident, $key:ty, $value:ty, $dual:expr, $fixed:expr, $int_key:expr
    ) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name;

        impl crate::hot::tables::Table for $name {
            const NAME: &'static str = stringify!($name);
            const FIXED_VAL_SIZE: Option<usize> = $fixed;
            const DUAL_KEY_SIZE: Option<usize> = $dual;
            const INT_KEY: bool = $int_key;
            type Key = $key;
            type Value = $value;
        }

    };

    // Single key
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
            None,
            false
        );

        impl crate::hot::tables::SingleKey for $name {}
    };

    // Single key with int_key
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $value:ty> int_key
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            None,
            None,
            true
        );

        impl crate::hot::tables::SingleKey for $name {}
    };

    // Dual key
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
            None,
            false
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
        }
    };

    // Dual key with int_key
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> int_key
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            None,
            true
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
        }
    };

    // Dual key with fixed size
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
            Some($fixed),
            false
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
        }
    };

    // Dual key with fixed size and int_key
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> is $fixed:expr, int_key
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            Some($fixed),
            true
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
        }
    };

    // Dual key with fixed size and mode
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> is $fixed:expr, $mode:ident
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            Some($fixed),
            false
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
            const TABLE_MODE: crate::hot::tables::DualTableMode =
                crate::hot::tables::DualTableMode::$mode;
        }
    };

    // Dual key with fixed size, mode, and int_key
    (
        #[doc = $doc:expr]
        $name:ident<$key:ty => $subkey:ty => $value:ty> is $fixed:expr, $mode:ident, int_key
    ) => {
        table!(@implement
            #[doc = $doc]
            $name,
            $key,
            $value,
            Some(<$subkey as crate::hot::ser::KeySer>::SIZE),
            Some($fixed),
            true
        );

        impl crate::hot::tables::DualKey for $name {
            type Key2 = $subkey;
            const TABLE_MODE: crate::hot::tables::DualTableMode =
                crate::hot::tables::DualTableMode::$mode;
        }
    };
}
