class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        return "Some sound";
    }

    async move() {
        console.log("Moving...");
    }
}

class Dog extends Animal {
    speak() {
        return "Woof!";
    }
}

export class Service {
    process(data) {
        return data;
    }
}
