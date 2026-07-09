//! `method { ... }` clause expansion for clauses without `py_args`.
//!
//! Split out of `builder_macros` to keep each module within the size limit.
//! The Python wrapper reuses the Rust argument list. Each rule emits the Rust
//! builder method and the matching Python wrapper, then recurses back into
//! `builder_methods!(@process_methods ...)`.

/// Expand a `method { ... }` clause whose Python wrapper reuses `rust_args`.
///
/// The four rules cover the combinations of optional `py_text_signature`
/// and `self_ident` fields. Clauses reach this macro via the fall-through
/// rule in `builder_method_clause!`.
macro_rules! builder_method_clause_rust_args {
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
                #[pyo3(signature = ( $( $rarg ),* ))]
                #[pyo3(text_signature = $py_text_signature)]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
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
                #[pyo3(signature = ( $( $rarg ),* ))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
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
                #[pyo3(signature = ( $( $rarg ),* ))]
                #[pyo3(text_signature = $py_text_signature)]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
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
                #[pyo3(signature = ( $( $rarg ),* ))]
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($rarg))* , ")"))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
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
}

pub(crate) use builder_method_clause_rust_args;
