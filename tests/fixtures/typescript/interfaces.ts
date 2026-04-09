interface Shape {
    area(): number;
    perimeter(): number;
}

interface Color {
    rgb: string;
    alpha: number;
}

type Point = {
    x: number;
    y: number;
};

type Vector = Point & { z: number };

enum Direction {
    Up = "UP",
    Down = "DOWN",
    Left = "LEFT",
    Right = "RIGHT",
}

class Circle implements Shape {
    constructor(public radius: number) {}

    area(): number {
        return Math.PI * this.radius * this.radius;
    }

    perimeter(): number {
        return 2 * Math.PI * this.radius;
    }
}
