// Starting from here the code is copy pasted for the rust standard library

use std::{any::TypeId, marker::PhantomData};

use crate::{thread_local::ThreadLocalActor, Actor, ActorCell, ActorRef, DerivedActorRef};

use super::Message;

pub(crate) struct DerivedProviderType<TActor>(PhantomData<fn() -> TActor>);
impl<T> DerivedProviderType<T> {
    pub(crate) fn new() -> Self {
        DerivedProviderType(PhantomData)
    }
}
pub(crate) struct DerivedProviderTypeLocal<TActor>(PhantomData<fn() -> TActor>);
impl<T> DerivedProviderTypeLocal<T> {
    pub(crate) fn new() -> Self {
        DerivedProviderTypeLocal(PhantomData)
    }
}

pub(crate) trait DerivedProvider: Send + Sync + 'static {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: &mut RequestDerived<'a>,
    );
}
impl<TActor: Actor> DerivedProvider for DerivedProviderType<TActor> {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: &mut RequestDerived<'a>,
    ) {
        TActor::provide_derived_actor_ref(ActorRef::<TActor::Msg>::from(actor_cell), request);
    }
}
impl<TActor: ThreadLocalActor> DerivedProvider for DerivedProviderTypeLocal<TActor> {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: &mut RequestDerived<'a>,
    ) {
        TActor::provide_derived_actor_ref(ActorRef::<TActor::Msg>::from(actor_cell), request);
    }
}
pub(crate) struct OwnedRequest<TMessage: 'static>(
    Tagged<TaggedOption<'static, tags::Value<DerivedActorRef<TMessage>>>>,
);
impl<TMessage: 'static> OwnedRequest<TMessage> {
    pub(crate) fn new() -> Self {
        Self(Tagged {
            tag_id: TypeId::of::<tags::Value<DerivedActorRef<TMessage>>>(),
            value: TaggedOption::<'static, tags::Value<DerivedActorRef<TMessage>>>(None),
        })
    }
    pub(crate) fn as_request(&mut self) -> &mut RequestDerived<'static> {
        self.0.as_request()
    }
    pub(crate) fn extract(self) -> Option<DerivedActorRef<TMessage>> {
        self.0.value.0
    }
}

/// Type used as argument to [Message::proved_derived_actor_ref]
#[repr(transparent)]
pub struct RequestDerived<'a>(Tagged<dyn Erased<'a> + 'a>);

impl<'a> RequestDerived<'a> {
    /// Provides a derived actor
    pub fn provide_derived_actor<T: Message>(&mut self, derived: DerivedActorRef<T>) -> &mut Self {
        self.provide_value(derived)
    }
    /// Provides a value or other type with only static lifetimes.
    ///
    fn provide_value<T>(&mut self, value: T) -> &mut Self
    where
        T: 'static,
    {
        self.provide::<tags::Value<T>>(value)
    }

    ///// Provides a value or other type with only static lifetimes computed using a closure.
    /////
    //fn provide_value_with<T>(&mut self, fulfil: impl FnOnce() -> T) -> &mut Self
    //where
    //    T: 'static,
    //{
    //    self.provide_with::<tags::Value<T>>(fulfil)
    //}

    ///// Provides a reference. The referee type must be bounded by `'static`,
    ///// but may be unsized.
    /////
    //fn provide_ref<T: ?Sized + 'static>(&mut self, value: &'a T) -> &mut Self {
    //    self.provide::<tags::Ref<tags::MaybeSizedValue<T>>>(value)
    //}

    ///// Provides a reference computed using a closure. The referee type
    ///// must be bounded by `'static`, but may be unsized.
    /////
    //fn provide_ref_with<T: ?Sized + 'static>(
    //    &mut self,
    //    fulfil: impl FnOnce() -> &'a T,
    //) -> &mut Self {
    //    self.provide_with::<tags::Ref<tags::MaybeSizedValue<T>>>(fulfil)
    //}

    /// Provides a value with the given `Type` tag.
    fn provide<I>(&mut self, value: I::Reified) -> &mut Self
    where
        I: tags::Type<'a>,
    {
        if let Some(res @ TaggedOption(None)) = self.0.downcast_mut::<I>() {
            res.0 = Some(value);
        }
        self
    }

    ///// Provides a value with the given `Type` tag, using a closure to prevent unnecessary work.
    //fn provide_with<I>(&mut self, fulfil: impl FnOnce() -> I::Reified) -> &mut Self
    //where
    //    I: tags::Type<'a>,
    //{
    //    if let Some(res @ TaggedOption(None)) = self.0.downcast_mut::<I>() {
    //        res.0 = Some(fulfil());
    //    }
    //    self
    //}

    ///// Checks if the `Request` would be satisfied if provided with a
    ///// value of the specified type. If the type does not match or has
    ///// already been provided, returns false.
    /////
    //fn would_be_satisfied_by_value_of<T>(&self) -> bool
    //where
    //    T: 'static,
    //{
    //    self.would_be_satisfied_by::<tags::Value<T>>()
    //}

    ///// Checks if the `Request` would be satisfied if provided with a
    ///// reference to a value of the specified type.
    /////
    //fn would_be_satisfied_by_ref_of<T>(&self) -> bool
    //where
    //    T: ?Sized + 'static,
    //{
    //    self.would_be_satisfied_by::<tags::Ref<tags::MaybeSizedValue<T>>>()
    //}

    //fn would_be_satisfied_by<I>(&self) -> bool
    //where
    //    I: tags::Type<'a>,
    //{
    //    matches!(self.0.downcast::<I>(), Some(TaggedOption(None)))
    //}
}

impl std::fmt::Debug for RequestDerived<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").finish_non_exhaustive()
    }
}

///////////////////////////////////////////////////////////////////////////////
// Type tags
///////////////////////////////////////////////////////////////////////////////

pub(crate) mod tags {
    //! Type tags are used to identify a type using a separate value. This module includes type tags
    //! for some very common types.
    //!
    //! Currently type tags are not exposed to the user. But in the future, if you want to use the
    //! Request API with more complex types (typically those including lifetime parameters), you
    //! will need to write your own tags.

    use core::marker::PhantomData;

    /// This trait is implemented by specific tag types in order to allow
    /// describing a type which can be requested for a given lifetime `'a`.
    ///
    /// A few example implementations for type-driven tags can be found in this
    /// module, although crates may also implement their own tags for more
    /// complex types with internal lifetimes.
    pub(crate) trait Type<'a>: Sized + 'static {
        /// The type of values which may be tagged by this tag for the given
        /// lifetime.
        type Reified: 'a;
    }

    /// Similar to the [`Type`] trait, but represents a type which may be unsized (i.e., has a
    /// `?Sized` bound). E.g., `str`.
    pub(crate) trait MaybeSizedType<'a>: Sized + 'static {
        type Reified: 'a + ?Sized;
    }

    impl<'a, T: Type<'a>> MaybeSizedType<'a> for T {
        type Reified = T::Reified;
    }

    /// Type-based tag for types bounded by `'static`, i.e., with no borrowed elements.
    #[derive(Debug)]
    pub(crate) struct Value<T: 'static>(PhantomData<T>);

    impl<T: 'static> Type<'_> for Value<T> {
        type Reified = T;
    }

    /// Type-based tag similar to [`Value`] but which may be unsized (i.e., has a `?Sized` bound).
    #[derive(Debug)]
    pub(crate) struct MaybeSizedValue<T: ?Sized + 'static>(PhantomData<T>);

    impl<T: ?Sized + 'static> MaybeSizedType<'_> for MaybeSizedValue<T> {
        type Reified = T;
    }

    /// Type-based tag for reference types (`&'a T`, where T is represented by
    /// `<I as MaybeSizedType<'a>>::Reified`.
    #[derive(Debug)]
    pub(crate) struct Ref<I>(PhantomData<I>);

    impl<'a, I: MaybeSizedType<'a>> Type<'a> for Ref<I> {
        type Reified = &'a I::Reified;
    }
}

/// An `Option` with a type tag `I`.
///
/// Since this struct implements `Erased`, the type can be erased to make a dynamically typed
/// option. The type can be checked dynamically using `Tagged::tag_id` and since this is statically
/// checked for the concrete type, there is some degree of type safety.
#[repr(transparent)]
pub(crate) struct TaggedOption<'a, I: tags::Type<'a>>(pub Option<I::Reified>);

impl<'a, I: tags::Type<'a>> Tagged<TaggedOption<'a, I>> {
    pub(crate) fn as_request(&mut self) -> &mut RequestDerived<'a> {
        let erased = self as &mut Tagged<dyn Erased<'a> + 'a>;
        // SAFETY: transmuting `&mut Tagged<dyn Erased<'a> + 'a>` to `&mut Request<'a>` is safe since
        // `Request` is repr(transparent).
        unsafe { &mut *(erased as *mut Tagged<dyn Erased<'a>> as *mut RequestDerived<'a>) }
    }
}

/// Represents a type-erased but identifiable object.
///
/// This trait is exclusively implemented by the `TaggedOption` type.
///
/// # Safety
/// Not to be used outside herre??
unsafe trait Erased<'a>: 'a {}

unsafe impl<'a, I: tags::Type<'a>> Erased<'a> for TaggedOption<'a, I> {}

struct Tagged<E: ?Sized> {
    tag_id: TypeId,
    value: E,
}

impl<'a> Tagged<dyn Erased<'a> + 'a> {
    /// Returns some reference to the dynamic value if it is tagged with `I`,
    /// or `None` otherwise.
    //#[inline]
    //fn downcast<I>(&self) -> Option<&TaggedOption<'a, I>>
    //where
    //    I: tags::Type<'a>,
    //{
    //    if self.tag_id == TypeId::of::<I>() {
    //        // SAFETY: Just checked whether we're pointing to an I.
    //        Some(&unsafe { &*(self as *const Self).cast::<Tagged<TaggedOption<'a, I>>>() }.value)
    //    } else {
    //        None
    //    }
    //}

    /// Returns some mutable reference to the dynamic value if it is tagged with `I`,
    /// or `None` otherwise.
    #[inline]
    fn downcast_mut<I>(&mut self) -> Option<&mut TaggedOption<'a, I>>
    where
        I: tags::Type<'a>,
    {
        if self.tag_id == TypeId::of::<I>() {
            Some(
                // SAFETY: Just checked whether we're pointing to an I.
                &mut unsafe { &mut *(self as *mut Self).cast::<Tagged<TaggedOption<'a, I>>>() }
                    .value,
            )
        } else {
            None
        }
    }
}
