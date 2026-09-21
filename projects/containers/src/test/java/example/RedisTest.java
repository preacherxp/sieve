package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;
import org.testcontainers.containers.GenericContainer;
import org.testcontainers.utility.DockerImageName;

class RedisTest {
    @Test void storesAndReads() throws Exception {
        try (GenericContainer<?> redis = new GenericContainer<>(DockerImageName.parse("redis:7.2.16-alpine"))
                .withExposedPorts(6379)) {
            redis.start();
            try (Socket socket = new Socket(redis.getHost(), redis.getMappedPort(6379));
                 BufferedReader input = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.UTF_8));
                 BufferedWriter output = new BufferedWriter(new OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8))) {
                socket.setSoTimeout(5000);
                output.write("*3\r\n$3\r\nSET\r\n$5\r\nquote\r\n$7\r\nEUR:120\r\n");
                output.flush();
                assertEquals("+OK", input.readLine());
                output.write("*2\r\n$3\r\nGET\r\n$5\r\nquote\r\n");
                output.flush();
                assertEquals("$7", input.readLine());
                assertEquals("EUR:120", input.readLine());
            }
        }
    }
}
