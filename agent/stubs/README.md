Compile-time stand-ins for the JUnit Platform and Spring TestContext types the agent uses.
They declare only the members the agent calls; the real libraries on the test classpath
provide them at run time. `build.rs` compiles against them and never packages them.
