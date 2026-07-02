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
    type Model;
    type ActiveModel;

    /// Convert a SeaORM model to a domain entity.
    fn to_domain(model: Self::Model) -> Domain;

    /// Convert a domain entity to a SeaORM ActiveModel for database writes.
    fn to_active_model(domain: &Domain) -> Self::ActiveModel;
}
