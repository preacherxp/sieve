package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import hierarchy.ParentConfig;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.test.context.ContextConfiguration;
import org.springframework.test.context.ContextHierarchy;
import org.springframework.test.context.junit.jupiter.SpringExtension;

@ExtendWith(SpringExtension.class)
@ContextHierarchy({
    @ContextConfiguration(classes = ParentConfig.class),
    @ContextConfiguration(classes = HierarchyTest.Child.class)
})
class HierarchyTest {
    @Configuration
    static class Child {
        @Bean
        String sentence(String word) {
            return word + "-child";
        }
    }

    @Autowired
    String sentence;

    @Test
    void combinesBothLevels() {
        assertEquals("parent-child", sentence);
    }
}
