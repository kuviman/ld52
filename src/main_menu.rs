use super::*;

struct ConfigScreen {
    assets: Rc<Assets>,
    geng: Geng,
    config: RoomConfig,
    addr: String,
    transition: Option<geng::Transition>,
}

impl ConfigScreen {
    fn new(geng: &Geng, assets: Rc<Assets>, addr: &str) -> Self {
        Self {
            assets,
            addr: addr.to_owned(),
            geng: geng.clone(),
            config: RoomConfig {
                seed: thread_rng().gen(),
                size: vec2(10, 10),
                image: 0,
            },
            transition: None,
        }
    }
}

impl geng::State for ConfigScreen {
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        ugli::clear(framebuffer, Some(Rgba::BLACK), None, None);
    }
    fn ui<'a>(&'a mut self, cx: &'a geng::ui::Controller) -> Box<dyn geng::ui::Widget + 'a> {
        use geng::ui::*;
        let play_button = Button::new(cx, "play");
        if play_button.was_clicked() {
            let future = {
                let geng = self.geng.clone();
                let addr = self.addr.clone();
                let config = self.config.clone();
                async move {
                    let mut con: Connection = geng::net::client::connect(&addr).await;
                    con.send(ClientMessage::CreateRoom(config));
                    let room = match con.next().await {
                        Some(ServerMessage::RoomCreated(name)) => name,
                        _ => unreachable!(),
                    };
                    game::run(&geng, &addr, &room)
                }
            };
            let state =
                geng::LoadingScreen::new(&self.geng, geng::EmptyLoadingScreen, future, |state| {
                    state
                });
            self.transition = Some(geng::Transition::Switch(Box::new(state)));
        }
        let image_button = Button::new(cx, &format!("Harvest #{}", self.config.image + 1));
        if image_button.was_clicked() {
            self.config.image = (self.config.image + 1) % self.assets.images.len();
        }
        (image_button, play_button).column().center().boxed()
    }
    fn transition(&mut self) -> Option<geng::Transition> {
        self.transition.take()
    }
}

pub fn run(geng: &Geng, addr: &str) -> impl geng::State {
    let future = {
        let geng = geng.clone();
        let addr = addr.to_owned();
        async move {
            ConfigScreen::new(
                &geng,
                geng::LoadAsset::load(&geng, &run_dir().join("assets"))
                    .await
                    .unwrap(),
                &addr,
            )
        }
    };
    geng::LoadingScreen::new(geng, geng::EmptyLoadingScreen, future, |state| state)
}