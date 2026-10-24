//! Helpers for binding SeaORM [`ActiveEnum`] values as PostgreSQL enum expressions.
//!
//! Plain `Expr::value(MyEnum::Variant)` serializes as `text`, which Postgres rejects
//! for enum columns (`operator does not exist: enum_x = text` / `column is of type
//! enum but expression is of type text`). Call [`pg_enum`] so SeaORM emits
//! `CAST($1 AS enum_…)`. Prefer this over `.as_enum()` at call sites: sea-query's
//! [`ExprTrait::as_enum`] takes a type name and shadows SeaORM's zero-arg method.

use sea_orm::{ActiveEnum, sea_query::SimpleExpr};

/// PURPOSE: Turn an ActiveEnum into a PG-typed `SimpleExpr` for UPDATE/CASE/SET.
/// USE: Pass to `col_expr` / `Expr::col(...).eq(...)` instead of `Expr::value(enum)`.
#[inline]
pub fn pg_enum<E: ActiveEnum>(value: &E) -> SimpleExpr {
    ActiveEnum::as_enum(value)
}
