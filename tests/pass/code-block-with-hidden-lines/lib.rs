//! This is an example showcasing code blocks with hidden lines
//!
//! Random lines with # will not be altered.
//!
//! - The same goes for lines in lists starting with a sharp:
//! - # this line is not hidden
//!
//! ```
//! // But in code blocks, lines starting with a # are hidden:
//! # // You can't see me
//!
//! // But not when those lines are actually relevant:
//! #[no_std]
//! ```
//!
//! ```ignore
//! // If the code block has "ignore" flag, it is not processed, and the lines starting with "#" are not hidden
//! # // You can see me
//! ```
