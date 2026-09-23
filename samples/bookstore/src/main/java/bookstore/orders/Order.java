package bookstore.orders;

import bookstore.catalog.Book;
import java.math.BigDecimal;

public record Order(String customer, Book book, int quantity, BigDecimal total) {}
