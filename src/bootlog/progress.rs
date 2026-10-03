//! Bounded step counter for the optional progress indicator (§30).

#[derive(Debug, Clone)]
pub struct Progress { total: usize, done: usize }

impl Progress {
    pub fn new(total: usize) -> Self { Self { total: total.max(1), done: 0 } }
    pub fn set_done(&mut self, done: usize) { self.done = done.min(self.total); }
    pub fn complete(&mut self) { self.done = self.total; }
    pub fn percent(&self) -> u8 {
        ((self.done as f32 / self.total as f32) * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percent_clamps() {
        let mut p = Progress::new(4);
        assert_eq!(p.percent(), 0);
        p.set_done(2);
        assert_eq!(p.percent(), 50);
        p.set_done(99);
        assert_eq!(p.percent(), 100);
        let mut p0 = Progress::new(0);
        assert_eq!(p0.percent(), 0);
        p0.complete();
        assert_eq!(p0.percent(), 100);
    }
}