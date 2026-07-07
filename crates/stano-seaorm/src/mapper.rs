/// Trait for bidirectional mapping between domain entities and SeaORM ActiveModels.
///
/// Applications implement this trait for each domain type that persists to the database.
///
/// # Example
///
/// ```ignore
/// use stano_seaorm::Mapper;
///
/// impl Mapper<MyDomain> for MyDomainMapper {
///     fn to_domain(model: MySeaOrmModel) -> MyDomain {
///         MyDomain {
///             id: MyId::from(model.id),
///             name: model.name,
///         }
///     }
///
///     fn to_active_model(domain: &MyDomain) -> MySeaOrmActiveModel {
///         MySeaOrmActiveModel {
///             id: Set(*domain.id.as_uuid()),
///             name: Set(domain.name.clone()),
///         }
///     }
/// }
/// ```
pub trait Mapper<Domain> {
    /// The SeaORM entity model this mapper reads from.
    type Model;
    /// The SeaORM `ActiveModel` this mapper writes to.
    type ActiveModel;

    /// Convert a SeaORM model to a domain entity.
    fn to_domain(model: Self::Model) -> Domain;

    /// Convert a domain entity to a SeaORM ActiveModel for database writes.
    fn to_active_model(domain: &Domain) -> Self::ActiveModel;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct UserDomain {
        id: i32,
        name: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct UserModel {
        id: i32,
        name: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct UserActiveModel {
        id: i32,
        name: String,
    }

    struct UserMapper;

    impl Mapper<UserDomain> for UserMapper {
        type Model = UserModel;
        type ActiveModel = UserActiveModel;

        fn to_domain(model: Self::Model) -> UserDomain {
            UserDomain {
                id: model.id,
                name: model.name,
            }
        }

        fn to_active_model(domain: &UserDomain) -> Self::ActiveModel {
            UserActiveModel {
                id: domain.id,
                name: domain.name.clone(),
            }
        }
    }

    #[test]
    fn test_to_domain_maps_fields() {
        let model = UserModel {
            id: 1,
            name: "Ada".to_string(),
        };
        let domain = UserMapper::to_domain(model);
        assert_eq!(domain.id, 1);
        assert_eq!(domain.name, "Ada");
    }

    #[test]
    fn test_to_active_model_maps_fields() {
        let domain = UserDomain {
            id: 2,
            name: "Grace".to_string(),
        };
        let active_model = UserMapper::to_active_model(&domain);
        assert_eq!(active_model.id, 2);
        assert_eq!(active_model.name, "Grace");
    }

    #[test]
    fn test_round_trip_domain_to_active_model_to_domain() {
        let domain = UserDomain {
            id: 3,
            name: "Katherine".to_string(),
        };
        let active_model = UserMapper::to_active_model(&domain);
        let model = UserModel {
            id: active_model.id,
            name: active_model.name,
        };
        let round_tripped = UserMapper::to_domain(model);
        assert_eq!(round_tripped, domain);
    }
}
