pub struct Point {
    pub x: f64,
    pub y: f64,
}

enum Direction {
    Up,
    Down,
    Left,
    Right,
}

pub trait Shape {
    fn area(&self) -> f64;
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl Shape for Point {
    fn area(&self) -> f64 {
        0.0
    }
}

const PI: f64 = 3.14159;
type MyResult<T> = Result<T, String>;
mod geometry {}
