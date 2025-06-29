use crate::resource::Resource;
use crate::system::parameter::SystemParam;
use crate::world::World;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub trait Command {
    fn execute(self: Box<Self>, world: &mut World);
}

pub struct CreateResource<T: Resource> {
    pub resource: T,
}

impl<T: Resource> Command for CreateResource<T> {
    fn execute(self: Box<Self>, world: &mut World) {
        world.insert_resource(self.resource);
    }
}

#[derive(Default)]
pub struct CommandQueue {
    pub commands: VecDeque<Box<dyn Command>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: VecDeque::new(),
        }
    }

    pub fn push(&mut self, command: Box<dyn Command>) {
        self.commands.push_back(command);
    }

    pub fn drain(&mut self) -> impl Iterator<Item=Box<dyn Command>> + use < '_ > {
        self.commands.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

pub struct Commands {
    buffer: Rc<RefCell<VecDeque<Box<dyn Command>>>>,
}

impl Commands {
    pub fn push(&mut self, command: impl Command + 'static) {
        self.buffer.borrow_mut().push_back(Box::new(command));
    }

    pub fn insert_resource<T: Resource>(&mut self, resource: T) {
        self.buffer.borrow_mut().push_back(Box::new(CreateResource { resource }));
    }
}

pub struct CommandsState {
    buffer: Rc<RefCell<VecDeque<Box<dyn Command>>>>,
}

impl SystemParam for Commands {
    type State = CommandsState;
    type Item<'world, 'state> = Commands;

    fn init_state(_: &mut World) -> Self::State {
        CommandsState {
            buffer: Rc::new(RefCell::new(VecDeque::new())),
        }
    }

    fn get_param<'world, 'state>(state: &'state Self::State, _: &'world mut World) -> Self::Item<'world, 'state> {
        Commands {
            buffer: Rc::clone(&state.buffer),
        }
    }

    fn apply_buffers(state: &Self::State, world: &mut World) {
        let mut buffer = state.buffer.borrow_mut();
        for command in buffer.drain(..) {
            world.add_command(command);
        }
    }
}