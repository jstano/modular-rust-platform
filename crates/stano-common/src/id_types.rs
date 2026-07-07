/// Generates a typed UUID newtype wrapper named `$name`.
///
/// Use `uuid_v4` for random IDs (nonces, transient identifiers); use `uuid_v7`
/// for sortable, time-ordered IDs (primary entity IDs) — `uuid_v7` additionally
/// derives `PartialOrd`/`Ord` since v7 UUIDs sort chronologically.
///
/// The generated type has `new()` (generates a fresh UUID), `from(Uuid)`,
/// `as_uuid(&self) -> &Uuid`, `FromStr`, and `Display`.
///
/// ```
/// stano_common::id_type!(UserId, uuid_v7);
/// let id = UserId::new();
/// assert_eq!(id, UserId::from(*id.as_uuid()));
/// ```
#[macro_export]
macro_rules! id_type {
    ($name:ident, uuid_v4) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub struct $name($crate::uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self($crate::uuid::Uuid::new_v4())
            }

            pub fn from(uuid: $crate::uuid::Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> &$crate::uuid::Uuid {
                &self.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self($crate::uuid::Uuid::parse_str(s)?))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
    ($name:ident, uuid_v7) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $name($crate::uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self($crate::uuid::Uuid::now_v7())
            }

            pub fn from(uuid: $crate::uuid::Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> &$crate::uuid::Uuid {
                &self.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self($crate::uuid::Uuid::parse_str(s)?))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use uuid::Uuid;

    // Define temporary id types for testing via the exported macro
    id_type!(TestIdV4, uuid_v4);
    id_type!(TestIdV7, uuid_v7);

    #[test]
    fn v4_new_produces_unique_ids_and_is_copy_eq_hash() {
        let mut set: HashSet<TestIdV4> = HashSet::new();
        for _ in 0..32 {
            let id = TestIdV4::new();
            assert!(set.insert(id), "duplicate UUID v4 generated unexpectedly");
        }

        let a = TestIdV4::new();
        let b = a; // copy
        assert_eq!(a, b);

        assert!(set.insert(a));
    }

    #[test]
    fn v4_from_and_as_uuid_and_display_round_trip() {
        let raw = Uuid::new_v4();
        let id = TestIdV4::from(raw);
        assert_eq!(id.as_uuid(), &raw);
        assert_eq!(id.to_string(), raw.to_string());
    }

    #[test]
    fn v7_new_produces_orderable_ids() {
        let mut ids: Vec<TestIdV7> = (0..32).map(|_| TestIdV7::new()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        ids.sort();
        assert_eq!(sorted, ids);
    }

    #[test]
    fn v7_from_and_as_uuid_and_display_round_trip() {
        let raw = Uuid::now_v7();
        let id = TestIdV7::from(raw);
        assert_eq!(id.as_uuid(), &raw);
        assert_eq!(id.to_string(), raw.to_string());
    }

    #[test]
    fn v4_from_str_round_trips_with_display() {
        let original = TestIdV4::new();
        let parsed: TestIdV4 = original.to_string().parse().unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn v4_from_str_invalid_string_returns_err() {
        assert!("not-a-uuid".parse::<TestIdV4>().is_err());
    }

    #[test]
    fn v7_from_str_round_trips_with_display() {
        let original = TestIdV7::new();
        let parsed: TestIdV7 = original.to_string().parse().unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn v7_from_str_invalid_string_returns_err() {
        assert!("not-a-uuid".parse::<TestIdV7>().is_err());
    }
}
