import { label } from "../src/format";

test("report", () => expect(label(5)).toBe("n=5"));
