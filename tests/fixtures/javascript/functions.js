/* biome-ignore-all lint/correctness/noUnusedVariables: every declaration is parser input */
function hello() {
    return "hello";
}

async function fetchData(url) {
    const response = await fetch(url);
    return response.json();
}

const compute = (x, y) => x + y;

const greet = (name) => `Hello, ${name}`;

module.exports = { hello, fetchData, compute, greet };
