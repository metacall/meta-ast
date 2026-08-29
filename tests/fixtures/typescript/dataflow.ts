// TypeScript fixture for dataflow extraction.
// Contains a mix of typed parameters, locals, calls, and returns so the
// extractor can capture both definitions and usages.

function add(a: number, b: number): number {
    return a + b;
}

function greet(name: string): string {
    const greeting = `Hello, ${name}`;
    return greeting;
}

class Counter {
    private count: number = 0;

    increment(by: number): number {
        this.count = this.count + by;
        return this.count;
    }

    getCount(): number {
        return this.count;
    }
}

function range(n: number): number[] {
    const result: number[] = [];
    let i = 0;
    while (i < n) {
        result.push(i);
        i = i + 1;
    }
    return result;
}
