use crate::component::ComponentBundle;
use crate::resource::Resource;
use crate::schedule::ScheduleLabel;
use crate::system::parameter::SystemParam;
use crate::system::{IntoSystem, System};
use crate::world::World;
use log::trace;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

// TODO: Does this have to be a Box<Self>?
pub trait Command {
    fn execute(self: Box<Self>, world: &mut World);
}

pub struct CreateResource<T: Resource> {
    pub resource: T,
}

impl<T: Resource> Command for CreateResource<T> {
    fn execute(self: Box<Self>, world: &mut World) {
        world.add_resource(self.resource);
    }
}

pub struct RemoveResource<T: Resource> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Resource> Command for RemoveResource<T> {
    fn execute(self: Box<Self>, world: &mut World) {
        world.remove_resource::<T>();
    }
}

pub struct RunSystem {
    pub system: Box<dyn System>,
}

// TODO: Could possibly be improved, verify whether we need double boxing (input and output)
// TODO: Should the From trait be used for something like this?
impl RunSystem {
    pub fn from_system<M>(system: impl IntoSystem<M>) -> Self {
        trace!(
            "Creating system from marker: {0}",
            std::any::type_name::<M>()
        );
        RunSystem {
            system: Box::new(IntoSystem::into_system(system)),
        }
    }
    // TODO: Once we rework system storage we should also add an overload that fetches a pre-registered system
}

impl Command for RunSystem {
    fn execute(self: Box<Self>, world: &mut World) {
        trace!("Running system once");
        world.run_system_once(self.system)
    }
}

pub struct RunSchedule {
    pub schedule_label: ScheduleLabel,
}

impl RunSchedule {
    pub fn new(schedule_label: ScheduleLabel) -> Self {
        Self { schedule_label }
    }
}

impl Command for RunSchedule {
    fn execute(self: Box<Self>, world: &mut World) {
        trace!("Running schedule once");
        world.run_schedule(&self.schedule_label)
    }
}

pub struct SpawnEntity<C: ComponentBundle> {
    pub components: C,
}

impl<C: ComponentBundle> Command for SpawnEntity<C> {
    fn execute(self: Box<Self>, world: &mut World) {
        world.spawn(self.components);
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

    pub fn drain(&mut self) -> impl Iterator<Item = Box<dyn Command>> + use<'_> {
        self.commands.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

// TODO: Verify whether RC is needed here
pub struct Commands {
    buffer: Rc<RefCell<VecDeque<Box<dyn Command>>>>,
}

impl Commands {
    pub fn push(&mut self, command: impl Command + 'static) {
        self.buffer.borrow_mut().push_back(Box::new(command));
    }

    pub fn insert_resource<T: Resource>(&mut self, resource: T) {
        self.buffer
            .borrow_mut()
            .push_back(Box::new(CreateResource { resource }));
    }

    pub fn remove_resource<T: Resource>(&mut self) {
        self.buffer
            .borrow_mut()
            .push_back(Box::new(RemoveResource::<T> {
                _phantom: std::marker::PhantomData,
            }));
    }

    pub fn run_system_once<M>(&mut self, into_system: impl IntoSystem<M>) {
        self.buffer
            .borrow_mut()
            .push_back(Box::new(RunSystem::from_system(into_system)))
    }

    pub fn run_schedule_once(&mut self, schedule_label: ScheduleLabel) {
        self.buffer
            .borrow_mut()
            .push_back(Box::new(RunSchedule::new(schedule_label)))
    }
    
    pub fn spawn<C: ComponentBundle + 'static>(&mut self, components: C) {
        self.buffer
            .borrow_mut()
            .push_back(Box::new(SpawnEntity { components }))
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

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        _: &'world mut World,
    ) -> Self::Item<'world, 'state> {
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
