// SPDX-License-Identifier: AGPL-3.0-only

pub mod cli;
pub mod convert;
pub mod error;
pub mod platform;
pub mod records;
pub mod save;

pub use error::{Error, Result};
pub use platform::Platform;
pub use save::{
    ConvertOptions, ConvertReport, SaveInfo, SaveSlot, SlotAction, convert_save, inspect_save,
    validate_output_path,
};
