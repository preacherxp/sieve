package example;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.sql.DriverManager;
import org.junit.jupiter.api.Test;
import org.testcontainers.postgresql.PostgreSQLContainer;

class PostgreSQLTest {
    @Test void commitsAndReadsRow() throws Exception {
        try (PostgreSQLContainer postgres = new PostgreSQLContainer("postgres:17-alpine")) {
            postgres.start();
            try (var connection = DriverManager.getConnection(
                    postgres.getJdbcUrl(), postgres.getUsername(), postgres.getPassword());
                 var statement = connection.createStatement()) {
                statement.execute("CREATE TABLE quotes (id INTEGER PRIMARY KEY, total INTEGER NOT NULL)");
                statement.executeUpdate("INSERT INTO quotes VALUES (1, 120)");
                try (var rows = statement.executeQuery("SELECT total FROM quotes WHERE id = 1")) {
                    assertTrue(rows.next());
                    assertEquals(120, rows.getInt(1));
                }
            }
        }
    }
}
