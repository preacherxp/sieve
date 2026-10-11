import { add } from "./math";
import labels from "./labels.json";

export const label = (n: number): string => `${labels.prefix}${add(n, 0)}`;
