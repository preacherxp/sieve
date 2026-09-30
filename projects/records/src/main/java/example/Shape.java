package example;

public interface Shape {
    int area();

    default String describe() {
        return "area " + area();
    }
}
