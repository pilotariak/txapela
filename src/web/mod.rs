// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The public pages outside the API: the landing page.

mod landing;

use worker::Router;

/// Registers the web routes on the router.
pub(crate) fn routes(router: Router<'_, ()>) -> Router<'_, ()> {
    router.get("/", |_, _| landing::get())
}
