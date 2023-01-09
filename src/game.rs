use geng::Camera2d;

use crate::jigsaw::Jigsaw;

use super::*;

const SNAP_DISTANCE: f32 = 0.2;
const FOV_MIN: f32 = 2.0;
const FOV_MAX: f32 = 20.0;

#[derive(HasId)]
struct Player {
    id: Id,
    interpolation: Interpolated<Vec2<f32>>,
    tile_grabbed: Option<(usize, Vec2<f32>)>,
}

struct Game {
    geng: Geng,
    room_config: RoomConfig,
    assets: Rc<Assets>,
    id: Id,
    connection: Connection,
    players: Collection<Player>,
    camera: Camera2d,
    framebuffer_size: Vec2<usize>,
    jigsaw: Jigsaw,
    bounds: AABB<f32>,
    dragging: Option<Dragging>,
    play_connect_sound: bool,
    intro_time: f32,
    time: f32,
    hovered_tile: Option<usize>,
}

#[derive(Debug, Clone)]
struct Dragging {
    pub initial_screen_pos: Vec2<f64>,
    pub target: DragTarget,
}

#[derive(Debug, Clone)]
enum DragTarget {
    Camera { initial_camera_pos: Vec2<f32> },
}

impl Game {
    pub fn new(
        geng: &Geng,
        assets: &Rc<Assets>,
        id: Id,
        room_config: RoomConfig,
        connection: Connection,
    ) -> Self {
        let image = &assets.images[room_config.image];
        let size = image.size().map(|x| x as f32);
        let size = size * 5.0 / size.y;
        let seed = room_config.seed;
        let mut jigsaw = Jigsaw::generate(geng.ugli(), seed, size, room_config.size);
        let mut rng = rand::prelude::StdRng::seed_from_u64(seed);
        let bounds = AABB::ZERO.extend_symmetric(size / 2.0).extend_uniform(3.0);
        let spawn_area =
            AABB::point(bounds.bottom_left()).extend_positive(vec2(bounds.width(), 3.0));
        for tile in &mut jigsaw.tiles {
            tile.interpolated
                .teleport(tile.interpolated.get() - size / 2.0, Vec2::ZERO);
            let pos = vec2(
                rng.gen_range(spawn_area.x_min..=spawn_area.x_max),
                rng.gen_range(spawn_area.y_min..=spawn_area.y_max),
            );
            tile.interpolated.server_update(pos, Vec2::ZERO);
        }
        Self {
            geng: geng.clone(),
            assets: assets.clone(),
            id,
            connection,
            players: Collection::new(),
            camera: Camera2d {
                center: Vec2::ZERO,
                rotation: 0.0,
                fov: 10.0,
            },
            framebuffer_size: vec2(1, 1),
            dragging: None,
            hovered_tile: None,
            play_connect_sound: false,
            bounds,
            jigsaw,
            room_config,
            intro_time: 1.0,
            time: 0.0,
        }
    }
    fn get_player(&mut self, id: Id) -> &mut Player {
        if self.players.get(&id).is_none() {
            self.players.insert(Player {
                id,
                interpolation: Interpolated::new(Vec2::ZERO, Vec2::ZERO),
                tile_grabbed: None,
            });
        }
        self.players.get_mut(&id).unwrap()
    }
    fn handle_connection(&mut self) {
        while let Some(message) = self.connection.try_recv() {
            match message {
                ServerMessage::SetupId(..) => unreachable!(),
                ServerMessage::RoomNotFound => unreachable!(),
                ServerMessage::RoomCreated(..) => unreachable!(),
                ServerMessage::UpdatePos(id, pos) => {
                    self.get_player(id)
                        .interpolation
                        .server_update(pos, Vec2::ZERO);
                }
                ServerMessage::PlayerDisconnected(id) => {
                    self.players.remove(&id);
                }
                ServerMessage::TileGrabbed {
                    player,
                    tile,
                    offset,
                } => {
                    self.players.get_mut(&player).unwrap().tile_grabbed = Some((tile, offset));
                    self.jigsaw.tiles[tile].grabbed_by = Some(player);
                    for tile in self.jigsaw.get_all_connected(tile) {
                        self.jigsaw.tiles[tile].last_interaction_time = self.time;
                    }
                }
                ServerMessage::TileReleased { player, tile, pos } => {
                    let player = self.get_player(player);
                    let offset = player
                        .tile_grabbed
                        .take()
                        .map_or(Vec2::ZERO, |(_, offset)| offset);
                    let vel = Some(player.interpolation.get_derivative());
                    self.jigsaw.tiles[tile].grabbed_by = None;
                    self.move_tile(tile, self.jigsaw.tiles[tile].interpolated.get(), vel, true);
                    self.move_tile(tile, pos + offset, None, false);
                }
                ServerMessage::ConnectTiles(a, b) => {
                    self.jigsaw.tiles[a].connected_to.push(b);
                    self.jigsaw.tiles[b].connected_to.push(a);
                    let delta = self.jigsaw.tiles[a].puzzle_pos.map(|x| x as i32)
                        - self.jigsaw.tiles[b].puzzle_pos.map(|x| x as i32);
                    let pos = if delta.x == 0 && delta.y.abs() == 1 {
                        // Tile is adjacent vertically
                        self.jigsaw.tiles[b].interpolated.get()
                            + vec2(0.0, self.jigsaw.tile_size.y * delta.y.signum() as f32)
                    } else if delta.y == 0 && delta.x.abs() == 1 {
                        // Tile is adjacent horizontally
                        self.jigsaw.tiles[b].interpolated.get()
                            + vec2(self.jigsaw.tile_size.x * delta.x.signum() as f32, 0.0)
                    } else {
                        unreachable!()
                    };
                    self.move_tile(a, pos, None, false);
                    self.play_connect_sound = true;
                }
            }
        }
    }
    fn hovered_tile(&self, pos: Vec2<f32>) -> Option<usize> {
        self.jigsaw
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, tile)| tile.contains(pos))
            .max_by_key(|(_, tile)| r32(tile.last_interaction_time))
            .map(|(i, _)| i)
    }
    fn click(&mut self, pos: Vec2<f32>) {
        if let Some(i) = self.hovered_tile(pos) {
            let tile = self.jigsaw.tiles.get_mut(i).unwrap();
            let player = self.players.get_mut(&self.id).unwrap();
            let offset = tile.interpolated.get() - pos;
            player.tile_grabbed = Some((i, offset));
            tile.grabbed_by = Some(self.id);
            for tile in self.jigsaw.get_all_connected(i) {
                self.jigsaw.tiles[tile].last_interaction_time = self.time;
            }
            self.assets.sounds.grab.play();
            self.connection
                .send(ClientMessage::GrabTile { tile: i, offset });
        }
    }
    fn release(&mut self) {
        self.stop_drag();
        let player = self.players.get_mut(&self.id).unwrap();
        if let Some((tile_id, _)) = player.tile_grabbed.take() {
            self.assets.sounds.grab.play();
            self.connection.send(ClientMessage::ReleaseTile(
                tile_id,
                player.interpolation.get(),
            ));
            let tile = self.jigsaw.tiles.get_mut(tile_id).unwrap();
            tile.grabbed_by = None;

            // Try to connect
            let connected = self.jigsaw.get_all_connected(tile_id);
            for tile_id in connected {
                let tile = self.jigsaw.tiles.get(tile_id).unwrap();
                let pos = tile.interpolated.get();
                let puzzle_pos = tile.puzzle_pos;
                for (i, other) in self.jigsaw.tiles.iter().enumerate() {
                    if tile.connected_to.contains(&i) {
                        continue;
                    }
                    let delta = puzzle_pos.map(|x| x as i32) - other.puzzle_pos.map(|x| x as i32);
                    let delta = if delta.x == 0 && delta.y.abs() == 1 {
                        // Tile is adjacent vertically
                        Some(
                            pos - other.interpolated.get()
                                - vec2(0.0, self.jigsaw.tile_size.y * delta.y.signum() as f32),
                        )
                    } else if delta.y == 0 && delta.x.abs() == 1 {
                        // Tile is adjacent horizontally
                        Some(
                            pos - other.interpolated.get()
                                - vec2(self.jigsaw.tile_size.x * delta.x.signum() as f32, 0.0),
                        )
                    } else {
                        None
                    };
                    if let Some(delta) = delta {
                        // Delta to the snap position
                        if delta.len() <= SNAP_DISTANCE {
                            self.connection
                                .send(ClientMessage::ConnectTiles(tile_id, i));
                        }
                    }
                }
            }
        }
    }
    fn move_tile(&mut self, tile: usize, pos: Vec2<f32>, vel: Option<Vec2<f32>>, snap: bool) {
        let vel = vel.unwrap_or(Vec2::ZERO);
        let tiles = self.jigsaw.get_all_connected(tile);
        let start_pos = self.jigsaw.tiles[tile].puzzle_pos.map(|x| x as i32);
        for tile in tiles {
            let delta = self.jigsaw.tiles[tile].puzzle_pos.map(|x| x as i32) - start_pos;
            if snap {
                self.jigsaw.tiles[tile]
                    .interpolated
                    .teleport(pos + delta.map(|x| x as f32) * self.jigsaw.tile_size, vel);
            } else {
                self.jigsaw.tiles[tile]
                    .interpolated
                    .server_update(pos + delta.map(|x| x as f32) * self.jigsaw.tile_size, vel);
            }
        }
    }
    fn start_drag(&mut self, drag: Dragging) {
        self.stop_drag();
        self.dragging = Some(drag);
    }
    fn update_cursor(&mut self, screen_pos: Vec2<f64>) {
        let cursor_pos = self
            .camera
            .screen_to_world(
                self.framebuffer_size.map(|x| x as f32),
                screen_pos.map(|x| x as f32),
            )
            .clamp_aabb(self.bounds);
        self.connection.send(ClientMessage::UpdatePos(cursor_pos));
        let me = self.get_player(self.id);
        me.interpolation.teleport(cursor_pos, Vec2::ZERO);

        if let Some(dragging) = &mut self.dragging {
            self.hovered_tile = None;
            match dragging.target {
                DragTarget::Camera { initial_camera_pos } => {
                    let from = self.camera.screen_to_world(
                        self.framebuffer_size.map(|x| x as f32),
                        dragging.initial_screen_pos.map(|x| x as f32),
                    );
                    let target = initial_camera_pos + from - cursor_pos;
                    self.camera.center = target.clamp_aabb(self.bounds);
                }
            }
        } else if let Some(hovered) = self.hovered_tile(cursor_pos) {
            if Some(hovered) != self.hovered_tile {
                self.hovered_tile = Some(hovered);
                // self.assets.sounds.hover.play();
            }
        } else {
            self.hovered_tile = None;
        }
    }
    fn stop_drag(&mut self) {
        if let Some(_dragging) = self.dragging.take() {}
    }
}

impl geng::State for Game {
    fn update(&mut self, delta_time: f64) {
        let delta_time = delta_time as f32;
        self.time += delta_time;

        self.handle_connection();

        if std::mem::take(&mut self.play_connect_sound) {
            self.assets.sounds.connect_piece.play();
        }

        let mut moves = Vec::new();
        for player in &mut self.players {
            player.interpolation.update(delta_time);

            // Update grabbed tile
            if let Some((tile_id, offset)) = player.tile_grabbed {
                if let Some(tile) = self.jigsaw.tiles.get_mut(tile_id) {
                    if tile.grabbed_by != Some(player.id) {
                        player.tile_grabbed = None;
                    } else {
                        moves.push((tile_id, player.interpolation.get() + offset));
                    }
                }
            }
        }
        for (tile, pos) in moves {
            self.move_tile(tile, pos, None, true);
        }

        if self.intro_time > 0.0 {
            self.intro_time -= delta_time;
        } else {
            for tile in &mut self.jigsaw.tiles {
                tile.interpolated.update(delta_time);
            }
        }
    }
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.framebuffer_size = framebuffer.size();
        ugli::clear(framebuffer, Some(Rgba::BLACK), None, None);

        self.geng.draw_2d(
            framebuffer,
            &self.camera,
            &draw_2d::Quad::new(self.bounds, Rgba::new(0.1, 0.1, 0.1, 1.0)),
        );

        let mut tiles: Vec<_> = self.jigsaw.tiles.iter().enumerate().collect();
        tiles.sort_by_key(|(_, tile)| r32(tile.last_interaction_time));
        let mut grabbed_tiles = HashMap::new();
        for (i, tile) in self.jigsaw.tiles.iter().enumerate() {
            if tile.grabbed_by.is_some() {
                for other in self.jigsaw.get_all_connected(i) {
                    grabbed_tiles.insert(other, tile);
                }
            }
        }
        for (i, tile) in tiles {
            let mut matrix = tile.matrix();
            if let Some(connected_to) = grabbed_tiles.get(&i) {
                let delta = (tile.puzzle_pos.map(|x| x as f32)
                    - connected_to.puzzle_pos.map(|x| x as f32))
                    * self.jigsaw.tile_size;
                matrix = connected_to.matrix() * Mat3::scale_uniform(1.2) * Mat3::translate(delta);
            }
            let outline_color = if Some(i) == self.hovered_tile {
                Rgba::WHITE
            } else {
                Rgba::BLACK
            };
            ugli::draw(
                framebuffer,
                &self.assets.shaders.outline,
                ugli::DrawMode::LineLoop { line_width: 2.0 },
                &tile.outline,
                (
                    ugli::uniforms! {
                        u_model_matrix: matrix,
                        u_color: outline_color,
                    },
                    geng::camera2d_uniforms(&self.camera, framebuffer.size().map(|x| x as f32)),
                ),
                ugli::DrawParameters::default(),
            );
            ugli::draw(
                framebuffer,
                &self.assets.shaders.jigsaw,
                ugli::DrawMode::Triangles,
                &tile.mesh,
                (
                    ugli::uniforms! {
                        u_model_matrix: matrix,
                        u_texture: &self.assets.images[self.room_config.image],
                    },
                    geng::camera2d_uniforms(&self.camera, framebuffer.size().map(|x| x as f32)),
                ),
                ugli::DrawParameters::default(),
            );
        }

        for player in &self.players {
            self.geng.draw_2d(
                framebuffer,
                &self.camera,
                &draw_2d::Ellipse::circle(
                    player.interpolation.get(),
                    self.camera.fov * 0.01,
                    Rgba::WHITE,
                ),
            );
        }
    }
    fn handle_event(&mut self, event: geng::Event) {
        match event {
            geng::Event::Wheel { delta } => {
                const SENSITIVITY: f32 = 0.02;
                self.camera.fov =
                    (self.camera.fov - delta as f32 * SENSITIVITY).clamp(FOV_MIN, FOV_MAX);
            }
            geng::Event::MouseMove { position, .. } => {
                self.update_cursor(position);
            }
            geng::Event::MouseDown { position, button } => match button {
                geng::MouseButton::Left => {
                    let pos = self.camera.screen_to_world(
                        self.framebuffer_size.map(|x| x as f32),
                        position.map(|x| x as f32),
                    );
                    self.click(pos);
                }
                geng::MouseButton::Right => {
                    self.start_drag(Dragging {
                        initial_screen_pos: position,
                        target: DragTarget::Camera {
                            initial_camera_pos: self.camera.center,
                        },
                    });
                }
                geng::MouseButton::Middle => {}
            },
            geng::Event::MouseUp { .. } => {
                self.release();
            }
            _ => (),
        }
    }
}

pub fn run(geng: &Geng, addr: &str, room: &str) -> impl geng::State {
    let future = {
        let geng = geng.clone();
        let room = room.to_owned();
        let connection = geng::net::client::connect(addr);
        async move {
            let assets: Rc<Assets> = geng::LoadAsset::load(&geng, &run_dir().join("assets"))
                .await
                .expect("Failed to load assets");
            let mut connection: game::Connection = connection.await;
            connection.send(ClientMessage::SelectRoom(room));
            match connection.next().await {
                Some(ServerMessage::SetupId(id, room_config)) => {
                    game::Game::new(&geng, &assets, id, room_config, connection)
                }
                Some(ServerMessage::RoomNotFound) => panic!("Room not found"),
                _ => unreachable!(),
            }
        }
    };
    geng::LoadingScreen::new(geng, geng::EmptyLoadingScreen, future, |state| state)
}
