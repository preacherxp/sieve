package example;

public class LoudGreeter extends Greeter {
    public String shout(String name) {
        return greet(name).toUpperCase();
    }
}
