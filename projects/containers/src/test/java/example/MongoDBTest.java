package example;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;

import com.mongodb.client.MongoClients;
import com.mongodb.client.model.Filters;
import org.bson.Document;
import org.junit.jupiter.api.Test;
import org.testcontainers.mongodb.MongoDBContainer;

class MongoDBTest {
    @Test void storesAndFindsDocument() {
        try (MongoDBContainer mongo = new MongoDBContainer("mongo:7.0.43").withReplicaSet()) {
            mongo.start();
            try (var client = MongoClients.create(mongo.getConnectionString())) {
                var quotes = client.getDatabase("sieve").getCollection("quotes");
                quotes.insertOne(new Document("_id", "quote").append("total", 120));
                Document quote = quotes.find(Filters.eq("_id", "quote")).first();
                assertNotNull(quote);
                assertEquals(120, quote.getInteger("total"));
            }
        }
    }
}
