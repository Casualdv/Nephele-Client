use glam::{Mat4, Vec3, Vec4};

/// First-person free camera. `yaw` is in radians, 0 looking down -Z; positive
/// yaw turns right. `pitch` is clamped just short of straight up/down.
#[derive(Debug, Clone)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub aspect: f32,
    pub speed: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(8.5, 80.0, 8.5),
            yaw: 0.0,
            pitch: -0.35,
            fov_y: 70f32.to_radians(),
            near: 0.05,
            far: 512.0,
            aspect: 16.0 / 9.0,
            speed: 24.0,
        }
    }
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp).normalize_or_zero()
    }

    /// Forward direction flattened onto the XZ plane.
    pub fn forward_flat(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(sy, 0.0, -cy).normalize_or_zero()
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    /// Camera up, perpendicular to [`Camera::forward`] and [`Camera::right`].
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize_or_zero()
    }

    /// Right-handed look-at, built explicitly so it does not depend on the
    /// math crate's naming.
    pub fn view(&self) -> Mat4 {
        let forward = self.forward();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward);
        Mat4::from_cols(
            Vec4::new(right.x, up.x, -forward.x, 0.0),
            Vec4::new(right.y, up.y, -forward.y, 0.0),
            Vec4::new(right.z, up.z, -forward.z, 0.0),
            Vec4::new(
                -right.dot(self.position),
                -up.dot(self.position),
                forward.dot(self.position),
                1.0,
            ),
        )
    }

    /// Right-handed perspective with a 0..1 depth range (wgpu/DirectX style).
    pub fn projection(&self) -> Mat4 {
        let aspect = self.aspect.max(0.01);
        let f = 1.0 / (self.fov_y * 0.5).tan();
        let range = self.near - self.far;
        Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, (self.far + self.near) / range, -1.0),
            Vec4::new(0.0, 0.0, (2.0 * self.far * self.near) / range, 0.0),
        )
    }

    pub fn view_projection(&self) -> Mat4 {
        self.projection() * self.view()
    }

    pub fn rotate(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw = (self.yaw + delta_yaw) % std::f32::consts::TAU;
        let limit = 89f32.to_radians();
        self.pitch = (self.pitch + delta_pitch).clamp(-limit, limit);
    }

    /// Move along the local axes; `forward`/`right`/`up` are -1..1.
    pub fn fly(&mut self, forward: f32, right: f32, up: f32, delta_seconds: f32) {
        let distance = self.speed * delta_seconds;
        let mut direction = self.forward_flat() * forward + self.right() * right;
        direction.y += up;
        self.position += direction * distance;
    }

    pub fn set_aspect(&mut self, width: u32, height: u32) {
        if height > 0 {
            self.aspect = width as f32 / height as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_forward_is_negative_z() {
        let camera = Camera {
            pitch: 0.0,
            yaw: 0.0,
            ..Camera::default()
        };
        let forward = camera.forward();
        assert!(forward.x.abs() < 1e-5);
        assert!(forward.y.abs() < 1e-5);
        assert!((forward.z + 1.0).abs() < 1e-5);
    }

    #[test]
    fn yaw_turns_right() {
        let camera = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            ..Camera::default()
        };
        let forward = camera.forward();
        assert!((forward.x - 1.0).abs() < 1e-5, "{forward:?}");
    }

    #[test]
    fn pitch_is_clamped() {
        let mut camera = Camera::default();
        camera.rotate(0.0, 10.0);
        assert!(camera.pitch < std::f32::consts::FRAC_PI_2);
        camera.rotate(0.0, -20.0);
        assert!(camera.pitch > -std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn view_projection_is_finite() {
        let camera = Camera::default();
        let matrix = camera.view_projection();
        assert!(matrix.to_cols_array().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn terrain_ahead_projects_into_the_ndc_box() {
        let camera = Camera {
            position: Vec3::new(8.5, 78.0, 8.5),
            yaw: 0.0,
            pitch: -0.35,
            ..Camera::default()
        };
        let clip = camera.view_projection() * glam::Vec4::new(8.5, 70.0, 0.0, 1.0);
        assert!(clip.w > 0.0, "point should be in front: {clip:?}");
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() <= 1.0, "x off screen: {ndc:?}");
        assert!(ndc.y.abs() <= 1.0, "y off screen: {ndc:?}");
        assert!((0.0..=1.0).contains(&ndc.z), "depth off range: {ndc:?}");
    }

    #[test]
    fn strafing_stays_level() {
        let mut camera = Camera {
            pitch: 0.0,
            ..Camera::default()
        };
        let start_y = camera.position.y;
        camera.fly(0.0, 1.0, 0.0, 1.0);
        assert!((camera.position.y - start_y).abs() < 1e-5);
    }
}
