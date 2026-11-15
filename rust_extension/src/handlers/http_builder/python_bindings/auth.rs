//! Validation of the Python `with_auth` mapping for [`HTTPHandlerBuilder`].
//!
//! The mapping describes either a bearer token or a user name and password
//! pair. Keeping the parsing here leaves the binding itself a plain dispatch
//! over the two accepted shapes.
//!
//! [`HTTPHandlerBuilder`]: super::HTTPHandlerBuilder

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// The mutually exclusive HTTP authentication shapes a mapping may describe.
pub(super) enum AuthSpec {
    /// A bearer token, used on its own.
    Bearer(String),
    /// A username and password pair.
    Basic {
        /// The user name to authenticate as.
        username: String,
        /// The password that accompanies the user name.
        password: String,
    },
}

/// Read and validate a `with_auth` mapping.
///
/// A token may not be combined with a user name or password, and the pair must
/// be supplied together, so a partly specified mapping fails here rather than
/// silently producing an unauthenticated handler.
pub(super) fn parse_auth_spec(config: &Bound<'_, PyDict>) -> PyResult<AuthSpec> {
    let token = config.get_item("token")?;
    let username = config.get_item("username")?;
    let password = config.get_item("password")?;
    match (token, username, password) {
        (Some(token), None, None) => Ok(AuthSpec::Bearer(token.extract()?)),
        (Some(_), _, _) => Err(pyo3::exceptions::PyValueError::new_err(
            "with_auth config must not mix 'token' with 'username'/'password'",
        )),
        (None, Some(username), Some(password)) => Ok(AuthSpec::Basic {
            username: username.extract()?,
            password: password.extract()?,
        }),
        (None, _, _) => Err(pyo3::exceptions::PyValueError::new_err(
            "with_auth config must specify either 'token' or both 'username' and 'password'",
        )),
    }
}
