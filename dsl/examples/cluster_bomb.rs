use rand::distributions::{Distribution, Uniform};
use rand::Rng;
use rayon::iter::ParallelIterator;
use specs_dsl::{
    data_item,
    specs::{
        Builder, Component, DenseVecStorage, DispatcherBuilder, Entities, Entity, HashMapStorage, Join, LazyUpdate,
        ParJoin, Read, ReadStorage, VecStorage, World, WorldExt, WriteStorage,
    },
    system, SystemDataType,
};

const TAU: f32 = 2. * std::f32::consts::PI;

#[derive(Component, Debug)]
#[storage(HashMapStorage)]
struct ClusterBomb {
    fuse: usize,
}

#[derive(Component, Debug)]
#[storage(HashMapStorage)]
struct Shrapnel {
    durability: usize,
}

#[derive(Component, Debug, Clone)]
#[storage(VecStorage)]
pub struct Pos(f32, f32);

#[derive(Component, Debug)]
#[storage(DenseVecStorage)]
pub struct Vel(f32, f32);

#[data_item]
#[system_data(PosChangeData)]
pub struct PosChange<'a> {
    pub position: &'a mut Pos,
    pub velocity: &'a Vel,
}

struct PhysicsSystem;

#[system(PosChangeData)]
impl PhysicsSystem {
    #[run]
    fn change_pos(&mut self, mut data: SystemDataType<Self>) {
        data.view_mut().par_join().for_each(|item| {
            let mut item: PosChange = item.into();

            item.position.0 += item.velocity.0;
            item.position.1 += item.velocity.1;
        });
    }
}

#[data_item]
struct BombChange<'a> {
    #[entity]
    entity: Entity,
    bomb: &'a mut ClusterBomb,
    position: &'a Pos,
}

type ClusterBombSystemData<'a> = (
    Entities<'a>,
    WriteStorage<'a, ClusterBomb>,
    ReadStorage<'a, Pos>,
    Read<'a, LazyUpdate>,
);

struct ClusterBombSystem;

#[system(ClusterBombSystemData)]
impl ClusterBombSystem {
    #[run]
    fn boom(&mut self, (entities, mut bombs, positions, updater): SystemDataType<Self>) {
        let durability_range = Uniform::new(10, 20);
        // Join components in potentially parallel way using rayon.
        (&entities, &mut bombs, &positions).par_join().for_each(|item| {
            let item: BombChange = item.into();
            let mut rng = rand::thread_rng();

            if item.bomb.fuse == 0 {
                let _ = entities.delete(item.entity);
                for _ in 0..9 {
                    let shrapnel = entities.create();
                    updater.insert(
                        shrapnel,
                        Shrapnel {
                            durability: durability_range.sample(&mut rng),
                        },
                    );
                    updater.insert(shrapnel, item.position.clone());
                    let angle: f32 = rng.gen::<f32>() * TAU;
                    updater.insert(shrapnel, Vel(angle.sin(), angle.cos()));
                }
            } else {
                item.bomb.fuse -= 1;
            }
        });
    }
}

#[data_item]
#[system_data(ShrapnelChangeData)]
struct ShrapnelChange<'a> {
    entity: Entity,
    shrapnel: &'a mut Shrapnel,
}

struct ShrapnelSystem;

#[system(ShrapnelChangeData)]
impl ShrapnelSystem {
    #[run]
    fn change_shrapnel(&mut self, (entities, mut shrapnels): SystemDataType<Self>) {
        (&entities, &mut shrapnels).par_join().for_each(|item| {
            let item: ShrapnelChange = item.into();

            if item.shrapnel.durability == 0 {
                let _ = entities.delete(item.entity);
            } else {
                item.shrapnel.durability -= 1;
            }
        });
    }
}

fn main() {
    let mut world = World::new();

    let mut dispatcher = DispatcherBuilder::new()
        .with(PhysicsSystem, "physics", &[])
        .with(ClusterBombSystem, "cluster_bombs", &[])
        .with(ShrapnelSystem, "shrapnels", &[])
        .build();

    dispatcher.setup(&mut world);

    world
        .create_entity()
        .with(Pos(0., 0.))
        .with(ClusterBomb { fuse: 3 })
        .build();

    let mut step = 0;
    loop {
        step += 1;
        let mut entities = 0;
        {
            // Simple console rendering
            let positions = world.read_storage::<Pos>();
            const WIDTH: usize = 10;
            const HEIGHT: usize = 10;
            const SCALE: f32 = 1. / 4.;
            let mut screen = [[0; WIDTH]; HEIGHT];
            for entity in world.entities().join() {
                if let Some(pos) = positions.get(entity) {
                    let x = (pos.0 * SCALE + WIDTH as f32 / 2.).floor() as usize;
                    let y = (pos.1 * SCALE + HEIGHT as f32 / 2.).floor() as usize;
                    if x < WIDTH && y < HEIGHT {
                        screen[x][y] += 1;
                    }
                }
                entities += 1;
            }
            println!("Step: {}, Entities: {}", step, entities);
            for row in &screen {
                for cell in row {
                    print!("{}", cell);
                }
                println!();
            }
            println!();
        }
        if entities == 0 {
            break;
        }

        dispatcher.dispatch(&world);

        // Maintain dynamically added and removed entities in dispatch.
        // This is what actually executes changes done by `LazyUpdate`.
        world.maintain();
    }
}
