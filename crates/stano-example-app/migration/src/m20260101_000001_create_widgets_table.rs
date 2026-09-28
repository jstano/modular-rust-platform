//! Starter migration for the `widgets` table backing the `Widget` entity in
//! `stano-example-domain`, applied automatically by `stano-example-app`'s launcher on
//! startup via `Migrator::up`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Widgets::Table)
                    .col(ColumnDef::new(Widgets::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Widgets::Name).text().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Widgets::Table).to_owned())
            .await
    }
}

/// `Widgets::Table` (via `DeriveIden`) renders as the snake-cased *enum name* — so this
/// must be named `Widgets`, not `Widget`, to match `entity::Model`'s
/// `#[sea_orm(table_name = "widgets")]`.
#[derive(DeriveIden)]
enum Widgets {
    Table,
    Id,
    Name,
}
