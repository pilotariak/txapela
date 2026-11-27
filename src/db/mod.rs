// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! D1 queries, one module per table, returning `domain` types.
//!
//! The Worker reaches the `pilotariak-txapela` database through the `DB` binding of
//! wrangler.toml. `api` calls this module, and this module uses `domain`, never `api`. The tables
//! come with the resources that need them, each one in a migration of `database/migrations/`.
