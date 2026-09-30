package example;

import static org.mockito.BDDMockito.given;
import static org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get;
import static org.springframework.test.web.servlet.result.MockMvcResultMatchers.content;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.autoconfigure.web.servlet.WebMvcTest;
import org.springframework.test.context.bean.override.mockito.MockitoBean;
import org.springframework.test.web.servlet.MockMvc;

@WebMvcTest(GreetingController.class)
class GreetingControllerTest {
    @Autowired
    MockMvc mvc;

    @MockitoBean
    GreetingService service;

    @Test
    void greetsOverHttp() throws Exception {
        given(service.greeting("Ada")).willReturn("mocked");
        mvc.perform(get("/greet").param("name", "Ada")).andExpect(content().string("mocked"));
    }
}
