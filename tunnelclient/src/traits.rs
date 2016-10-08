use graphics::{Context, Graphics};
use config::ClientConfig;

pub trait Interpolate {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self;
}

impl<T: Interpolate + Clone> Interpolate for Vec<T> {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
        if self.len() != other.len() {
            if alpha < 0.5 {return self.clone()}
            else {return other.clone()}
        }
        self.iter()
            .zip(other.iter())
            .map(|(a, b)| a.interpolate_with(b, alpha))
            .collect::<Vec<_>>()
    }
}

pub trait Draw<G: Graphics> {
    /// Given a context and gl instance, draw this entity to the screen.
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig);
}

impl<T: Draw> Draw for Vec<T> {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        for e in self {
            e.draw(c, gl, cfg);
        }
    }
}