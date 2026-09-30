package example;

import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import org.junit.jupiter.api.Test;
import org.testcontainers.containers.GenericContainer;
import org.testcontainers.utility.DockerImageName;

/** Appends the ID of a reusable container, so that two runs can compare them. */
class ReusedContainerTest {
    @Test
    void startsAReusableContainer() throws IOException {
        GenericContainer<?> redis = new GenericContainer<>(DockerImageName.parse("redis:7.2.16-alpine"))
                .withExposedPorts(6379)
                .withReuse(true);
        redis.start();
        assertTrue(redis.isRunning());
        Files.writeString(Path.of("target/container-ids.txt"), redis.getContainerId() + "\n",
                StandardOpenOption.CREATE, StandardOpenOption.APPEND);
    }
}
