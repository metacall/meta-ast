import React from "react";
import { helper } from "./helper";

export const compute = (value) => helper(value);

export function render(element) {
    return React.createElement(element, { value: compute(1) });
}
