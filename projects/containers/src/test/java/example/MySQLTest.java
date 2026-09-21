package example;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.sql.DriverManager;
import org.junit.jupiter.api.Test;
import org.testcontainers.mysql.MySQLContainer;

class MySQLTest {
    @Test void storesAndReadsRow() throws Exception {
        try (MySQLContainer mysql = new MySQLContainer("mysql:8.4")) {
            mysql.start();
            try (var connection = DriverManager.getConnection(
                    mysql.getJdbcUrl(), mysql.getUsername(), mysql.getPassword());
                 var statement = connection.createStatement()) {
                statement.execute("CREATE TABLE quotes (id INTEGER PRIMARY KEY, currency VARCHAR(3) NOT NULL)");
                statement.executeUpdate("INSERT INTO quotes VALUES (1, 'EUR')");
                try (var rows = statement.executeQuery("SELECT currency FROM quotes WHERE id = 1")) {
                    assertTrue(rows.next());
                    assertEquals("EUR", rows.getString(1));
                }
            }
        }
    }
}
