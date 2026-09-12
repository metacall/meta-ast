function hello() {
    return "hello";
}

async function fetchData(url) {
    const response = await fetch(url);
    return response.json();
}

const compute = (x, y) => x + y;

const greet = (name) => `Hello, ${name}`;

export async function demo(url) {
    const data = await fetchData(url);
    return [hello(), data, compute(1, 2), greet("world")];
}
