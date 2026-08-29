// TSX fixture for dataflow extraction.
// Exercises typed parameters, locals, and arrow functions
// so the dataflow extractor can identify both definitions and uses.

interface Props {
    name: string;
}

function Greet(props: Props): JSX.Element {
    const message = `Hello, ${props.name}`;
    return <div>{message}</div>;
}

const Welcome = (props: Props): JSX.Element => {
    return <span>{props.name}</span>;
};

function makeGreeting(name: string): string {
    const greeting = `Hi, ${name}`;
    return greeting;
}
