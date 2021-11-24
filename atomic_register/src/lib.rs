use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

pub struct AtomicRegister<T> {
    atomic_ref: AtomicPtr<T>,
}

impl<T> AtomicRegister<T> {
    pub fn new(init: T) -> Arc<AtomicRegister<T>> {
        let inner = Box::leak(Box::new(init));
        let atomic_ref = AtomicPtr::new(inner);
        Arc::new(AtomicRegister { atomic_ref })
    }

    pub fn write(&self, new_value: T) {
        let new_inner = Box::leak(Box::new(new_value));
        let _old_ptr = self.atomic_ref.swap(new_inner, Ordering::SeqCst);

        // Drop the old inner
        //
        // FIXME: Dropping the value soon makes read() impossible. You must use some mechanism to
        // ensure the other thread is currently accessing the value to the pointer. For example, you
        // can use hazard pointer. For now, we leak the memory to keep the value always alive.
        // let _ = unsafe { Box::from_raw(old_ptr) };
    }
}

impl<T> AtomicRegister<T>
where
    T: Clone,
{
    // You can't make it return &T: we need to ensure the backing value is alive during the reading
    // from backing pointer, but it can't.
    pub fn read(&self) -> T {
        // First we need to register the current pointer to the hazard pointer, then read the value,
        // and unregister from the hazard pointer.
        // unimplemented!("how to lock-freely implement this?")

        // For now, we leaks the all heap memory. We can safely read from the pointer.
        let ptr = self.atomic_ref.load(Ordering::SeqCst);
        // SAFETY: currently the pointer is always valid, as it is leaked from Box.
        unsafe { (&*ptr).clone() }
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
}
