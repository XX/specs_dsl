use rand::Rng;
use rand::distributions::{Distribution, Uniform};
use rayon::iter::ParallelIterator;
use specs_dsl::{
    data_item, system, SystemDataType,
    specs::{
        Component, VecStorage, DenseVecStorage, HashMapStorage, Entities, Builder, DispatcherBuilder, World, WorldExt,
        System, Read, ReadStorage, WriteStorage, LazyUpdate, Join, ParJoin,
    }
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
#[system_data(ChangePosData)]
pub struct ChangePos<'a> {
    pub pos: &'a mut Pos,
    pub vel: &'a Vel,
}

struct PhysicsSystem;

#[system(ChangePosData)]
impl PhysicsSystem {
    #[run]
    fn change_pos(&mut self, mut data: SystemDataType<Self>) {
        data.view_mut().par_join().for_each(|item| {
            let mut item: ChangePos = item.into();

            item.pos.0 += item.vel.0;
            item.pos.1 += item.vel.1;
        });
    }
}


struct ClusterBombSystem;

impl<'a> System<'a> for ClusterBombSystem {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, ClusterBomb>,
        ReadStorage<'a, Pos>,
        // Allows lazily adding and removing components to entities
        // or executing arbitrary code with world access lazily via `execute`.
        Read<'a, LazyUpdate>,
    );

    fn run(&mut self, (entities, mut bombs, positions, updater): Self::SystemData) {
        let durability_range = Uniform::new(10, 20);
        // Join components in potentially parallel way using rayon.
        (&entities, &mut bombs, &positions)
            .par_join()
            .for_each(|(entity, bomb, position)| {
                let mut rng = rand::thread_rng();

                if bomb.fuse == 0 {
                    let _ = entities.delete(entity);
                    for _ in 0..9 {
                        let shrapnel = entities.create();
                        updater.insert(
                            shrapnel,
                            Shrapnel {
                                durability: durability_range.sample(&mut rng),
                            },
                        );
                        updater.insert(shrapnel, position.clone());
                        let angle: f32 = rng.gen::<f32>() * TAU;
                        updater.insert(shrapnel, Vel(angle.sin(), angle.cos()));
                    }
                } else {
                    bomb.fuse -= 1;
                }
            });
    }
}

struct ShrapnelSystem;

impl<'a> System<'a> for ShrapnelSystem {
    type SystemData = (Entities<'a>, WriteStorage<'a, Shrapnel>);

    fn run(&mut self, (entities, mut shrapnels): Self::SystemData) {
        (&entities, &mut shrapnels)
            .par_join()
            .for_each(|(entity, shrapnel)| {
                if shrapnel.durability == 0 {
                    let _ = entities.delete(entity);
                } else {
                    shrapnel.durability -= 1;
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
