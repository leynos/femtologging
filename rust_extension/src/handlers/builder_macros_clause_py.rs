//! `method { ... }` clause expansion for clauses that provide `py_args`.
//!
//! Split out of `builder_macros` to keep each module within the size limit.
//! Each rule emits the Rust builder method and the matching Python wrapper,
//! then recurses back into `builder_methods!(@process_methods ...)`. Clauses
//! without `py_args` fall through to `builder_method_clause_rust_args!`.

/// Expand a single `method { ... }` clause forwarded by `builder_methods!`.
///
/// The first four rules handle clauses with `py_args` (with or without
/// `py_text_signature` and `self_ident`); the final rule forwards every
/// other clause shape to [`builder_method_clause_rust_args!`].
macro_rules! builder_method_clause {
    (
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        method {
            doc: $doc:expr,
            rust_name: $rust_name:ident,
            py_fn: $py_fn:ident,
            py_name: $py_name:literal,
            py_text_signature: $py_text_signature:literal,
            rust_args: ( $( $rarg:ident : $rty:ty ),* ),
            py_args: ( $( $parg:ident : $pty:ty ),* ),
            $(py_prelude: { $($py_prelude:tt)* },)?
            body: $body:block
        },
        remaining = [ $($rest:tt)* ],
        extra = [ $($extra:tt)* ]
    ) => {
        $crate::handlers::builder_macros::builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                $crate::handlers::builder_macros::builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), builder, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                #[pyo3(text_signature = $py_text_signature)]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let builder = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $($extra)*
        );
    };

    (
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        method {
            doc: $doc:expr,
            rust_name: $rust_name:ident,
            py_fn: $py_fn:ident,
            py_name: $py_name:literal,
            rust_args: ( $( $rarg:ident : $rty:ty ),* ),
            py_args: ( $( $parg:ident : $pty:ty ),* ),
            $(py_prelude: { $($py_prelude:tt)* },)?
            body: $body:block
        },
        remaining = [ $($rest:tt)* ],
        extra = [ $($extra:tt)* ]
    ) => {
        $crate::handlers::builder_macros::builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                $crate::handlers::builder_macros::builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), builder, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($parg))* , ")"))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let builder = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $($extra)*
        );
    };

    (
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        method {
            doc: $doc:expr,
            rust_name: $rust_name:ident,
            py_fn: $py_fn:ident,
            py_name: $py_name:literal,
            py_text_signature: $py_text_signature:literal,
            rust_args: ( $( $rarg:ident : $rty:ty ),* ),
            py_args: ( $( $parg:ident : $pty:ty ),* ),
            $(py_prelude: { $($py_prelude:tt)* },)?
            self_ident: $self_ident:ident,
            body: $body:block
        },
        remaining = [ $($rest:tt)* ],
        extra = [ $($extra:tt)* ]
    ) => {
        $crate::handlers::builder_macros::builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                $crate::handlers::builder_macros::builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), $self_ident, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                #[pyo3(text_signature = $py_text_signature)]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let $self_ident = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $($extra)*
        );
    };

    (
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        method {
            doc: $doc:expr,
            rust_name: $rust_name:ident,
            py_fn: $py_fn:ident,
            py_name: $py_name:literal,
            rust_args: ( $( $rarg:ident : $rty:ty ),* ),
            py_args: ( $( $parg:ident : $pty:ty ),* ),
            $(py_prelude: { $($py_prelude:tt)* },)?
            self_ident: $self_ident:ident,
            body: $body:block
        },
        remaining = [ $($rest:tt)* ],
        extra = [ $($extra:tt)* ]
    ) => {
        $crate::handlers::builder_macros::builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                $crate::handlers::builder_macros::builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), $self_ident, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($parg))* , ")"))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let $self_ident = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $($extra)*
        );
    };

    // Clauses without `py_args` are handled by the sibling macro.
    (
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        method { $($fields:tt)* },
        remaining = [ $($rest:tt)* ],
        extra = [ $($extra:tt)* ]
    ) => {
        $crate::handlers::builder_macros::builder_method_clause_rust_args!(
            $builder,
            [$($rust_methods)*],
            [$($py_methods)*],
            method { $($fields)* },
            remaining = [ $($rest)* ],
            extra = [ $($extra)* ]
        );
    };
}

pub(crate) use builder_method_clause;
