Compile-time stand-ins for the Maven core and JSR-330 types the extension uses. They declare
only the members the extension calls; Maven provides them at run time. `build.rs` compiles
against them and never packages them.
