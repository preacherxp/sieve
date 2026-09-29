package bookstore.catalog;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;

public class Catalog {
    private final Map<String, Book> books = new LinkedHashMap<>();

    public void add(Book book) {
        books.put(book.isbn(), book);
    }

    public Optional<Book> find(String isbn) {
        return Optional.ofNullable(books.get(isbn));
    }

    public List<Book> search(String text) {
        String query = text.toLowerCase(Locale.ROOT);
        return books.values().stream()
                .filter(b -> b.title().toLowerCase(Locale.ROOT).contains(query)
                        || b.author().toLowerCase(Locale.ROOT).contains(query))
                .toList();
    }
}
