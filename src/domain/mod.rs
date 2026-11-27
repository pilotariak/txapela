// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Pure logic and types: no I/O, no clock, and no `worker` import, so `cargo test` runs them on
//! the host.
//!
//! A rule that depends on the time takes it as an argument: the routes of `api` read the Worker's
//! clock and pass it in. This module uses neither `api` nor `db`.
