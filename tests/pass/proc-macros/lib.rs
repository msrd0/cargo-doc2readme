//! This cool crate has the following definitely very useful macros:
//!
//!  - [`test_macro`]
//!  - [`test_macro!`] (which is definitely different from the above)
//!  - [`test_attr`]
//!  - [`TestDerive`]

use proc_macro::TokenStream;

#[proc_macro]
pub fn test_macro(input: TokenStream) -> TokenStream {
	input
}

#[proc_macro_attribute]
pub fn test_attr(_attr: TokenStream, input: TokenStream) -> TokenStream {
	input
}

#[proc_macro_derive(TestDerive)]
pub fn test_derive(_input: TokenStream) -> TokenStream {
	Default::default()
}
