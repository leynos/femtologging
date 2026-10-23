//! Support types used by HTTP handler tests.

/// Request details captured by the local mock server for assertions.
#[derive(Debug)]
pub(super) struct CapturedRequest {
    /// HTTP method from the request line.
    pub(super) method: String,
    /// Request path from the request line.
    pub(super) path: String,
    /// Header names and values sent with the request.
    pub(super) headers: Vec<(String, String)>,
    /// Request body read using the declared content length.
    pub(super) body: String,
}
