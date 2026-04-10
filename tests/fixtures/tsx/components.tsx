import React from 'react';

interface Props {
    name: string;
    age: number;
}

function Greeting(props: Props): JSX.Element {
    return <div>Hello, {props.name}!</div>;
}

const Welcome: React.FC<Props> = (props) => {
    return <span>Welcome {props.name}</span>;
};

export class App extends React.Component<Props> {
    render() {
        return <Greeting name={this.props.name} age={this.props.age} />;
    }
}
