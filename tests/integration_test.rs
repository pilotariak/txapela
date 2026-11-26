// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for txapela.

#[test]
fn package_name_is_txapela() {
    assert_eq!(env!("CARGO_PKG_NAME"), "txapela");
}
