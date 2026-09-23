use std::marker::PhantomData;

pub struct Store<Id, Value>
where
    Id: Into<usize> + From<usize>,
{
    entries: Vec<Value>,
    marker: PhantomData<fn() -> Id>,
}

impl<Id, Value> Store<Id, Value>
where
    Id: Into<usize> + From<usize>,
{
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            marker: PhantomData,
        }
    }

    pub fn insert(&mut self, value: Value) -> Id {
        self.entries.push(value);
        Id::from(self.entries.len() - 1)
    }

    pub fn get(&self, id: Id) -> &Value {
        &self.entries[id.into()]
    }

    pub fn get_mut(&mut self, id: Id) -> &mut Value {
        &mut self.entries[id.into()]
    }
}
