package example;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;
import static org.junit.jupiter.api.Assertions.assertEquals;

class LabelTest {
    @ParameterizedTest
    @CsvSource({"alpha beta,label:ALPHA-BETA", "gamma,label:GAMMA"})
    void formats(String input, String expected) {
        assertEquals(expected, Label.of(input));
    }
}
