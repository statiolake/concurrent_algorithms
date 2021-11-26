use register::Register;
use thread_local::ThreadLocal;

pub struct Collect<T> {
    id: ThreadLocal<usize>,
    regs: Vec<Register<(usize, T)>>,
}

impl<T: Clone> Collect<T> {
    pub fn new(init: T, num_processes: usize) -> Collect<T> {
        let id = ThreadLocal::new();
        let regs = (0..num_processes)
            .map(|id| Register::new((id, init.clone())))
            .collect();

        Collect { id, regs }
    }

    pub fn collect(&self) -> Vec<T> {
        (0..self.regs.len())
            .map(|id| self.regs[id].read())
            .map(|(_, v)| v)
            .collect()
    }
}

impl<T> Collect<T> {
    pub fn store(&self, id: usize, new_value: T) {
        // Check ID validity
        if id >= self.regs.len() {
            panic!("id {} is beyond the len {}", id, self.regs.len());
        }
        let prev_id = *self.id.get_or(|| id);
        if prev_id != id {
            panic!("prev_id: {}, id: {}; id is not consistent!", prev_id, id);
        }

        // store value at position id
        self.regs[id].write((id, new_value));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use crate::Collect;

    #[test]
    fn simple_collect() {
        const NUM_PROCESSES: usize = 128;
        let collect = Arc::new(Collect::new(0, NUM_PROCESSES));

        // False-positive clippy lint for thread handle
        // see: https://github.com/rust-lang/rust-clippy/issues/7207
        #[allow(clippy::needless_collect)]
        let ths: Vec<_> = (0..NUM_PROCESSES)
            .map(|id| {
                let collect = Arc::clone(&collect);
                thread::spawn(move || {
                    collect.store(id, id);
                })
            })
            .collect();
        ths.into_iter().for_each(|th| th.join().unwrap());
        assert_eq!(collect.collect(), vec![0, 1]);
    }
}
