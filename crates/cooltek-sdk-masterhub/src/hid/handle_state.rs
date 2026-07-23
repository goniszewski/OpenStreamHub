use hidapi::HidDevice;

/// Represents a discrete state in the lifetime of a [`ModuleHandle`].
///
/// This is useful in allowing particular functions to only be called in specific states. The
/// compiler will prevent calls to functions when the instance is in an incorrect state, reducing
/// errors.
///
/// See the [Rust Book].
///
/// See [Rust Patterns & Engineering How-Tos].
///
/// [Rust Book]: https://doc.rust-lang.org/stable/book/ch18-03-oo-design-patterns.html#encoding-states-and-behavior-as-types
/// [Rust Patterns & Engineering How-Tos]: https://microsoft.github.io/RustTraining/rust-patterns-book/ch03-the-newtype-and-type-state-patterns.html
pub trait HandleState: crate::private::Sealed {
    /// The data structure associated with this state.
    #[doc(hidden)]
    type Data;
}

// -------------------------------

/// A type state indicating that the module is currently connected.
///
/// # Semantics
///
/// Conceptually, a user should be able to communicate with a "connected" module without errors
/// (ignoring I/O and other module-specific errors).
///
/// In practice, the state machine does not understand a "connected" module being automatically
/// disconnected (e.g. physically removed from the host). A "connected" module has a valid (if
/// stale) device handle that can attempt to transfer information. This kind of error **cannot**
/// be caught at compile-time.
///
/// In line with Rust conventions, we allow these operations and rely on users to handle `Err`
/// cases that may arise. Well-behaved users should either [`Drop`] the handle or call
/// [`disconnect()`][super::ModuleHandle::disconnect] manually.
pub struct Connected;

impl HandleState for Connected {
    // The "connected" state contains the actual device handle.
    //
    // This is safer than an `Option` on `ModuleHandle` because library developers cannot forget to
    // set the data. It also completely eliminates defensive checking if the option is `None`
    // before performing operations on the device handle.
    type Data = HidDevice;
}

impl crate::private::Sealed for Connected {}

// -------------------------------

/// A type state indicating that the module is currently disconnected.
pub struct Disconnected;

impl HandleState for Disconnected {
    // The "disconnected" state contains the unit type, which has no functions defined on it.
    type Data = ();
}

impl crate::private::Sealed for Disconnected {}
