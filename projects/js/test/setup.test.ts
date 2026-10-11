test("setup registers handlers", () =>
  expect((globalThis as { handlers?: string[] }).handlers).toEqual(["GET /prices"]));
