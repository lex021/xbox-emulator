pub(crate) struct W(pub Vec<u8>);

pub(crate) struct R<'a>(pub &'a [u8], pub usize);

impl<'a> R<'a> {
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.0.get(self.1..self.1.checked_add(n)?)?;
        self.1 += n;
        Some(b)
    }
}

pub(crate) trait Codec: Sized {
    fn put(&self, w: &mut W);
    fn get(r: &mut R) -> Option<Self>;
}

macro_rules! num_codec {
    ($($t:ty),*) => {$(
        impl Codec for $t {
            fn put(&self, w: &mut W) {
                w.0.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut R) -> Option<Self> {
                Some(<$t>::from_le_bytes(r.take(core::mem::size_of::<$t>())?.try_into().ok()?))
            }
        }
    )*};
}

num_codec!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl Codec for bool {
    fn put(&self, w: &mut W) {
        w.0.push(*self as u8);
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(r.take(1)?[0] != 0)
    }
}

impl<const N: usize> Codec for [u8; N] {
    fn put(&self, w: &mut W) {
        w.0.extend_from_slice(self);
    }
    fn get(r: &mut R) -> Option<Self> {
        r.take(N)?.try_into().ok()
    }
}

impl Codec for [u32; 3] {
    fn put(&self, w: &mut W) {
        for v in self {
            v.put(w);
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some([u32::get(r)?, u32::get(r)?, u32::get(r)?])
    }
}

impl<T: Codec> Codec for Vec<T> {
    fn put(&self, w: &mut W) {
        (self.len() as u32).put(w);
        for v in self {
            v.put(w);
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        let n = u32::get(r)? as usize;
        if n > r.0.len() {
            return None;
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(T::get(r)?);
        }
        Some(v)
    }
}

impl<T: Codec> Codec for Option<T> {
    fn put(&self, w: &mut W) {
        match self {
            None => w.0.push(0),
            Some(v) => {
                w.0.push(1);
                v.put(w);
            }
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        match r.take(1)?[0] {
            0 => Some(None),
            _ => Some(Some(T::get(r)?)),
        }
    }
}

impl<T: Codec> Codec for Box<T> {
    fn put(&self, w: &mut W) {
        (**self).put(w);
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(Box::new(T::get(r)?))
    }
}

impl<A: Codec, B: Codec> Codec for (A, B) {
    fn put(&self, w: &mut W) {
        self.0.put(w);
        self.1.put(w);
    }
    fn get(r: &mut R) -> Option<Self> {
        Some((A::get(r)?, B::get(r)?))
    }
}

macro_rules! struct_codec {
    ($name:ident { $($f:ident),* $(,)? }) => {
        impl $crate::netview::wfp_codec::Codec for $name {
            fn put(&self, w: &mut $crate::netview::wfp_codec::W) {
                $( $crate::netview::wfp_codec::Codec::put(&self.$f, w); )*
            }
            fn get(r: &mut $crate::netview::wfp_codec::R) -> Option<Self> {
                Some($name { $( $f: $crate::netview::wfp_codec::Codec::get(r)?, )* })
            }
        }
    };
}

pub(crate) use struct_codec;
