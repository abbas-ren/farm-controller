//! Build inventory and upload HTTP handlers under `/api/v1/device/build*`.
//!
//! Ports Node `buildRoutes.ts` / `BuildController` from `farm_controller_old/device-service`.
//! Frontend build library and upload flows; TUS lives in `tus_handlers`.

mod mutations;
mod queries;
mod uploads;

pub(crate) use mutations::*;
pub(crate) use queries::*;
pub(crate) use uploads::*;
