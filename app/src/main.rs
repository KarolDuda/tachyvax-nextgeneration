#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;

fn main() { api::run(); }
