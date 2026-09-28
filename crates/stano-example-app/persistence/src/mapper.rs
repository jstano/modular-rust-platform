//! Domain <-> SeaORM entity conversion for [`Widget`], per `stano-seaorm`'s `Mapper` seam.

use sea_orm::ActiveValue::Set;
use stano_example_domain::{Widget, WidgetId};
use stano_seaorm::Mapper;

use crate::entity;

pub struct WidgetMapper;

impl Mapper<Widget> for WidgetMapper {
    type Model = entity::Model;
    type ActiveModel = entity::ActiveModel;

    fn to_domain(model: Self::Model) -> Widget {
        Widget {
            id: WidgetId::from(model.id),
            name: model.name,
        }
    }

    fn to_active_model(domain: &Widget) -> Self::ActiveModel {
        entity::ActiveModel {
            id: Set(*domain.id.as_uuid()),
            name: Set(domain.name.clone()),
        }
    }
}
