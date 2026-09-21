package example;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;

import com.rabbitmq.client.ConnectionFactory;
import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;
import org.testcontainers.rabbitmq.RabbitMQContainer;

class RabbitMQTest {
    @Test void publishesAndReceives() throws Exception {
        try (RabbitMQContainer rabbit = new RabbitMQContainer("rabbitmq:4.1-alpine")) {
            rabbit.start();
            ConnectionFactory factory = new ConnectionFactory();
            factory.setUri(rabbit.getAmqpUrl());
            try (var connection = factory.newConnection(); var channel = connection.createChannel()) {
                String queue = channel.queueDeclare().getQueue();
                byte[] quote = "EUR:120".getBytes(StandardCharsets.UTF_8);
                channel.confirmSelect();
                channel.basicPublish("", queue, null, quote);
                channel.waitForConfirmsOrDie(5000);
                var received = channel.basicGet(queue, true);
                assertNotNull(received);
                assertArrayEquals(quote, received.getBody());
            }
        }
    }
}
