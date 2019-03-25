use js_sys::{Array, JsString, Map, Object, Reflect, Uint8Array};
use serde::ser::{self, Error as _, Serialize};
use wasm_bindgen::JsValue;

use super::{convert_error, Error, static_str_to_js};

type Result<T = JsValue> = super::Result<T>;

/// Wraps other serializers into an enum tagged variant form.
/// Uses {"Variant": ...payload...} for compatibility with serde-json.
pub struct VariantSerializer<S> {
    variant: &'static str,
    inner: S,
}

impl<S> VariantSerializer<S> {
    pub fn new(variant: &'static str, inner: S) -> Self {
        Self { variant, inner }
    }

    fn end(self, inner: impl FnOnce(S) -> Result) -> Result {
        let value = inner(self.inner)?;
        let obj = JsValue::from(Object::new());
        Reflect::set(&obj, &static_str_to_js(self.variant), &value).unwrap();
        Ok(obj)
    }
}

impl<S: ser::SerializeTupleStruct<Ok = JsValue, Error = Error>> ser::SerializeTupleVariant
    for VariantSerializer<S>
{
    type Ok = JsValue;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.inner.serialize_field(value)
    }

    fn end(self) -> Result {
        self.end(S::end)
    }
}

impl<S: ser::SerializeStruct<Ok = JsValue, Error = Error>> ser::SerializeStructVariant
    for VariantSerializer<S>
{
    type Ok = JsValue;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.inner.serialize_field(key, value)
    }

    fn end(self) -> Result {
        self.end(S::end)
    }
}

pub struct ArraySerializer<'s> {
    serializer: &'s Serializer,
    target: Array,
}

impl<'s> ArraySerializer<'s> {
    pub fn new(serializer: &'s Serializer) -> Self {
        Self {
            serializer,
            target: Array::new(),
        }
    }
}

impl ser::SerializeSeq for ArraySerializer<'_> {
    type Ok = JsValue;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.target.push(&value.serialize(self.serializer)?);
        Ok(())
    }

    fn end(self) -> Result {
        Ok(self.target.into())
    }
}

impl ser::SerializeTuple for ArraySerializer<'_> {
    type Ok = JsValue;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for ArraySerializer<'_> {
    type Ok = JsValue;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        ser::SerializeTuple::serialize_element(self, value)
    }

    fn end(self) -> Result {
        ser::SerializeTuple::end(self)
    }
}

pub struct MapSerializer<'s> {
    serializer: &'s Serializer,
    target: Map,
    next_key: Option<JsValue>,
}

impl<'s> MapSerializer<'s> {
    pub fn new(serializer: &'s Serializer) -> Self {
        Self {
            serializer,
            target: Map::new(),
            next_key: None,
        }
    }
}

impl ser::SerializeMap for MapSerializer<'_> {
    type Ok = JsValue;
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        debug_assert!(self.next_key.is_none());
        self.next_key = Some(key.serialize(self.serializer)?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.target.set(
            &self.next_key.take().unwrap(),
            &value.serialize(self.serializer)?,
        );
        Ok(())
    }

    fn end(self) -> Result {
        debug_assert!(self.next_key.is_none());
        Ok(self.target.into())
    }
}

pub struct ObjectSerializer<'s> {
    serializer: &'s Serializer,
    target: Object,
}

impl<'s> ObjectSerializer<'s> {
    pub fn new(serializer: &'s Serializer) -> Self {
        Self {
            serializer,
            target: Object::new(),
        }
    }
}

impl ser::SerializeStruct for ObjectSerializer<'_> {
    type Ok = JsValue;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        Reflect::set(
            &self.target,
            &static_str_to_js(key),
            &value.serialize(self.serializer)?,
        )
        .map_err(convert_error)?;
        Ok(())
    }

    fn end(self) -> Result {
        Ok(self.target.into())
    }
}

// Serializer might be configurable in the future, so add but hide its implementation details.
#[derive(Default)]
pub struct Serializer(());

impl Serializer {
    pub fn new() -> Self {
        Default::default()
    }
}

macro_rules! forward_to_into {
    ($($name:ident($ty:ty);)*) => {
        $(fn $name(self, v: $ty) -> Result {
            Ok(v.into())
        })*
    };
}

impl<'s> ser::Serializer for &'s Serializer {
    type Ok = JsValue;
    type Error = Error;

    type SerializeSeq = ArraySerializer<'s>;
    type SerializeTuple = ArraySerializer<'s>;
    type SerializeTupleStruct = ArraySerializer<'s>;
    type SerializeTupleVariant = VariantSerializer<ArraySerializer<'s>>;
    type SerializeMap = MapSerializer<'s>;
    type SerializeStruct = ObjectSerializer<'s>;
    type SerializeStructVariant = VariantSerializer<ObjectSerializer<'s>>;

    forward_to_into! {
        serialize_bool(bool);

        serialize_i8(i8);
        serialize_i16(i16);
        serialize_i32(i32);

        serialize_u8(u8);
        serialize_u16(u16);
        serialize_u32(u32);

        serialize_f32(f32);
        serialize_f64(f64);

        serialize_str(&str);
    }

    // TODO: we might want to support `BigInt` here in the future.
    fn serialize_i64(self, v: i64) -> Result {
        const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
        const MIN_SAFE_INTEGER: i64 = -MAX_SAFE_INTEGER;

        if v >= MIN_SAFE_INTEGER && v <= MAX_SAFE_INTEGER {
            self.serialize_f64(v as _)
        } else {
            Err(Error::custom(format_args!(
                "{} can't be represented as a JavaScript number",
                v
            )))
        }
    }

    // TODO: we might want to support `BigInt` here in the future.
    fn serialize_u64(self, v: u64) -> Result {
        const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

        if v <= MAX_SAFE_INTEGER {
            self.serialize_f64(v as _)
        } else {
            Err(Error::custom(format_args!(
                "{} can't be represented as a JavaScript number",
                v
            )))
        }
    }

    fn serialize_char(self, v: char) -> Result {
        Ok(JsValue::from(JsString::from_code_point1(v as u32).unwrap()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result {
        // Create a `Uint8Array` view into a Rust slice, and immediately copy it to the JS memory.
        //
        // This is necessary because any allocation in WebAssembly can require reallocation of the
        // backing memory, which will invalidate existing views (including `Uint8Array`).
        Ok(JsValue::from(Uint8Array::new(
            unsafe { Uint8Array::view(v) }.as_ref(),
        )))
    }

    fn serialize_none(self) -> Result {
        Ok(JsValue::UNDEFINED)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result {
        Ok(JsValue::UNDEFINED)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result {
        self.serialize_unit()
    }

    /// For compatibility with serde-json, serialises unit variants as "Variant" strings.
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result {
        Ok(static_str_to_js(variant))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result {
        VariantSerializer::new(variant, self.serialize_newtype_struct(variant, value)?).end(Ok)
    }

    /// Serialises any Rust iterable into a JS Array.
    // TODO: Figure out if there is a way to detect and serialise `Set` differently.
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
        Ok(ArraySerializer::new(self))
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.serialize_tuple(len)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Ok(VariantSerializer::new(
            variant,
            self.serialize_tuple_struct(variant, len)?,
        ))
    }

    /// Serialises Rust maps into JS `Map`.
    // TODO: We might want to support serialising maps with string keys to JS objects.
    // They are tricky to detect until Rust stabilises specialisation support.
    // Additionally, even if we can detect it, we might still choose to use the more
    // efficient `Map`, so this has to be a configuration option.
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        Ok(MapSerializer::new(self))
    }

    /// Serialises Rust typed structs into plain JS objects.
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
        Ok(ObjectSerializer::new(self))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Ok(VariantSerializer::new(
            variant,
            self.serialize_struct(variant, len)?,
        ))
    }
}
