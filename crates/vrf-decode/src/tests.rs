//! Tests ported from the C# reference:
//! - PrimitiveDecodersScalarTests.cs
//! - PrimitiveDecodersVectorTests.cs
//! - RepLayoutArrayDecodersTests.cs (structural only -- DynamicArray is Raw)
//!
//! The array, struct-blob and effect decoders keep their tests next to
//! their own modules; what lives here is the primitive decoders and the
//! overlay.

#[cfg(feature = "overlay")]
mod overlay;
mod scalar;
mod vector;
