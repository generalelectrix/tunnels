/// GPU-compatible vertex format. Position is in pixel coordinates, color is RGBA [0,1].
/// `repr(C)` and bytemuck derives are required for wgpu buffer uploads.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// Abstraction between tessellation and rendering.
///
/// Implemented by `FrameBuilder` (GPU path in show.rs) and `RenderBuffer` (software test path).
/// `clear()` resets the target. `draw_triangles()` submits indexed triangle geometry.
pub trait RenderTarget {
    fn clear(&mut self, color: [f32; 4]);
    fn draw_triangles(&mut self, vertices: &[Vertex], indices: &[u32]);
}
