// JavaScript fixture for dataflow extraction.
// Exercises parameter capture, local bindings, and call/binary usage sites
// so the dataflow extractor can identify both definitions and uses.

function add(a, b) {
    return a + b;
}

function increment(counter) {
    counter.value = counter.value + 1;
    return counter;
}

const make = (x) => {
    const doubled = x * 2;
    return doubled;
};

let total = 0;
function addToTotal(value) {
    total = total + value;
}
