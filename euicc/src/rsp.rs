// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of the parameter structs and enum es10b_cancel_session_reason from lpac
// euicc/es10b.h (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten as Rust types shared by the ES9+ (es9p) and ES10b download code.

//! Values passed between the operator's server (ES9+) and the card (ES10b) during a download.
//! Fields hold base64 text exactly as exchanged with the server, as in libeuicc.

/// From ES9+ InitiateAuthentication, for ES10b AuthenticateServer
/// (`struct es10b_authenticate_server_param`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthenticateServerParam {
    pub b64_server_signed1: String,
    pub b64_server_signature1: String,
    pub b64_euicc_ci_pkid_to_be_used: String,
    pub b64_server_certificate: String,
}

/// From ES9+ AuthenticateClient, for the preview and ES10b PrepareDownload
/// (`struct es10b_prepare_download_param`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepareDownloadParam {
    pub b64_profile_metadata: String,
    pub b64_smdp_signed2: String,
    pub b64_smdp_signature2: String,
    pub b64_smdp_certificate: String,
}

/// CancelSession reasons (`enum es10b_cancel_session_reason`, SGP.22 CancelSessionReason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSessionReason {
    EndUserRejection = 0,
    Postponed = 1,
    Timeout = 2,
    PprNotAllowed = 3,
    MetadataMismatch = 4,
    LoadBppExecutionError = 5,
    Undefined = 127,
}
