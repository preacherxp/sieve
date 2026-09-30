package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class ShapeTest {
    @Test
    void describesThroughTheInterface() {
        Shape shape = new Square(3);
        assertEquals("area 9", shape.describe());
    }
}
