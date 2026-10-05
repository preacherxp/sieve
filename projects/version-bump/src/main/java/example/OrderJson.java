package example;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.util.Map;

/** Order payloads, through Jackson. */
public final class OrderJson {
    private static final ObjectMapper MAPPER = new ObjectMapper();

    private OrderJson() {}

    public static String write(String id, long cents) throws JsonProcessingException {
        return MAPPER.writeValueAsString(Map.of("id", id, "cents", cents));
    }

    public static long cents(String json) throws JsonProcessingException {
        return MAPPER.readTree(json).get("cents").asLong();
    }
}
