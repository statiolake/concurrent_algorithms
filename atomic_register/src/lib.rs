use std::num::NonZeroUsize;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use thread_local::ThreadLocal;

pub struct AtomicRegister<T> {
    ptr: AtomicPtr<T>,

    hazard_ptr: ThreadLocal<AtomicPtr<T>>,

    // Here we use Mutex in order to enable mutation in each thread. Note that this is still lock
    // free, since the inner value won't seen from another thread except final Drop implementation
    // (so we can always aqcuire lock immediately).
    //
    // The type of list element should be *mut T, but that's not allowed because *mut T is not Send.
    // In order to avoid this, we first cast it to usize. The dereference is always safe since no
    // other process drop the pointee.
    drop_later: ThreadLocal<Mutex<Vec<NonZeroUsize>>>,
}

impl<T> AtomicRegister<T> {
    pub fn new(init: T) -> Arc<AtomicRegister<T>> {
        let inner = Box::leak(Box::new(init));
        let atomic_ref = AtomicPtr::new(inner);
        let hazard_ptr = ThreadLocal::new();
        let drop_later = ThreadLocal::new();
        Arc::new(AtomicRegister {
            ptr: atomic_ref,
            hazard_ptr,
            drop_later,
        })
    }

    pub fn write(&self, new_value: T) {
        let new_ptr = Box::leak(Box::new(new_value));

        // Write the new pointer to the atomic reference. new_ptr and old_ptr is always different,
        // since new_ptr is a new heap allocation just created above.
        let old_ptr = self.ptr.swap(new_ptr, Ordering::SeqCst);

        // Register the old pointer to the list of pointers which should be dropped later.
        self.drop_later_mut()
            .push(NonZeroUsize::new(old_ptr as _).expect("internal error: old_ptr was null"));

        // Drop the old values if it's no longer used.
        self.drop_if_unused();
    }

    fn drop_if_unused(&self) {
        self.drop_later_mut().retain(|ptr| {
            // Check if the ptr is in use
            let in_use = self
                .hazard_ptr
                .iter()
                .any(|hazptr| hazptr.load(Ordering::SeqCst) as usize == ptr.get());

            // Drop the value if it's not in use
            if !in_use {
                // SAFETY: ptr is registered in write() executed in this thread and no other thread
                // has this pointer in their drop_later list (due to atomicity of AtomicPtr::swap()
                // operation). Therefore other thread never drop the inner value, so ptr must be a
                // valid pointer.
                let _ = unsafe { Box::from_raw(ptr.get() as *mut T) };
            }

            // retain the value in drop_later if it's still in use
            in_use
        })
    }

    fn drop_later_mut(&self) -> MutexGuard<Vec<NonZeroUsize>> {
        self.drop_later
            .get_or_default()
            .try_lock()
            .expect("internal error: this lock should never fail")
    }
}

impl<T> AtomicRegister<T>
where
    T: Clone,
{
    // You can't make it return &T: we need to ensure the backing value is alive during the reading
    // from backing pointer, but it can't.
    pub fn read(&self) -> T {
        // We need to keep trying until we get an access to the valid pointee.
        let mut ptr = null_mut();
        loop {
            // First we need to get the current pointer and state I'll using the pointer.
            let check_ptr = self.ptr.load(Ordering::SeqCst);

            // Claim the current ptr is in use.
            self.hazard_ptr
                .get_or_default()
                .store(check_ptr, Ordering::SeqCst);

            // Check the current pointer is the same with the previous one. If current ptr and
            // prev_ptr is the same, this ptr is reserved by this thread --- we set this ptr in the
            // previous loop and the value is still alive. We can ensure no other thread drops the
            // value by the hazard pointer.
            // Note that, as known as ABA problem, we can't detect the change occured two times
            // (A->B->A). In this case, however, is fine since the hazard pointer guarantees the
            // pointee of A is valid. If two objects have the same address, the new object is kept
            // correctly.
            if check_ptr == ptr {
                break;
            }

            // Otherwise, if different, then it should be the following case:
            //
            // 1. We load prev_ptr in the previous run.
            // 2. Other thread writes new value and freed the value in prev_ptr.
            // 3. We register the prev_ptr to the hazard pointer.
            //
            // ... In other words, our registration was bit too late. Retry next run.
            ptr = check_ptr;
        }

        // SAFETY: our pointer is protected by the hazard pointer, so the pointee should alive at
        // this point.
        let value = unsafe { (&*ptr).clone() };

        // Finished using the pointer. Unregister the hazard pointer.
        let old_ptr = self
            .hazard_ptr
            .get()
            .expect("internal error: hazard_ptr must have the value")
            .swap(null_mut(), Ordering::SeqCst);

        assert_eq!(
            ptr, old_ptr,
            "The pointer was not protected by the hazard pointer!?"
        );

        value
    }
}

impl<T> Drop for AtomicRegister<T> {
    fn drop(&mut self) {
        // We need to drop the rest objects. That is the objects which couldn't be dropped because
        // of the protection by the hazard pointer when writing the new value to it.

        // First, check the all hazard pointer is null (or empty), since now no thread can access to
        // the AtomicRegister.
        assert!(
            self.hazard_ptr
                .iter()
                .all(|ptr| ptr.load(Ordering::SeqCst).is_null()),
            concat!(
                "Some threads still have non-null hazard pointer",
                "even when AtomicRegister is going to be dropped"
            )
        );

        // Second, drop all left pointers.
        self.drop_later
            .iter_mut()
            .flat_map(|v| {
                // This get_mut() should never fail, since no Mutex should be poisoned, but we can't
                // panic inside the drop implementation. Therefore we avoid unwrap() or expect() and
                // silently ignore errors using flat_map().
                v.get_mut()
            })
            .flatten()
            .for_each(|ptr| {
                // SAFETY: the pointer should never be freed and no pointer is duplicated in
                // different threads.
                unsafe {
                    let _ = Box::from_raw(ptr.get() as *mut T);
                }
            });
    }
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use crate::AtomicRegister;

    #[test]
    fn sequencial_read() {
        let reg = AtomicRegister::new(0);
        reg.write(10);
        assert_eq!(reg.read(), 10);
    }

    #[test]
    fn multi_thread_read() {
        let reg = Arc::new(AtomicRegister::new(0));
        let reg1 = Arc::clone(&reg);
        let reg2 = Arc::clone(&reg);

        let t1 = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            reg1.write(10);
            let value = reg1.read();
            assert!(value == 10 || value == 20);
        });
        let t2 = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            reg2.write(20);
            let value = reg2.read();
            assert!(value == 10 || value == 20);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let value = reg.read();
        assert!(value == 10 || value == 20);
    }

    #[test]
    fn massive_writes() {
        const COUNT: usize = 10000000;

        let reg = Arc::new(AtomicRegister::new(0));
        let reg1 = Arc::clone(&reg);
        let reg2 = Arc::clone(&reg);

        let t1 = thread::spawn(move || {
            for cnt in 1..=COUNT {
                reg1.write(cnt)
            }
        });

        let t2 = thread::spawn(move || {
            let mut old = 0;
            loop {
                let curr = reg2.read();
                assert!(curr >= old);
                old = curr;

                if curr == COUNT {
                    break;
                }
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let value = reg.read();
        assert_eq!(value, COUNT);
    }
}
