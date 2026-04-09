function hello() {
    return "hello";
}

async function fetchData(url) {
    const response = await fetch(url);
    return response.json();
}

const compute = function(x, y) {
    return x + y;
};

const greet = (name) => `Hello, ${name}`;
