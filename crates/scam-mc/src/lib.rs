//! Установка и запуск Minecraft.
//!
//! [`install::prepare`] скачивает всё нужное для версии (JSON версии, библиотеки, ассеты, Java),
//! [`launch::launch`] собирает командную строку и запускает игру.

pub mod dirs;
pub mod install;
pub mod java;
pub mod launch;
pub mod loader;
pub mod log;
pub mod net;
pub mod offline;
pub mod rules;
pub mod server;
pub mod version;

pub use dirs::GameDirs;
pub use net::{Mirrors, Net, Progress};
