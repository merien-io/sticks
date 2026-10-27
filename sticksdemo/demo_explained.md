
# Sticks 1.0 Architecture & Annotated `index.html`

This document details the mechanics of `public/index.html` as implemented for the Sticks 1.0 Task List application. It explains each block, its role in the execution lifecycle, how it adheres to the Sticks philosophy, and how it solves the **Rule S4 (No Nesting)** constraint.

---

## 1. High-Level Lifecycle & The Two Zones

Sticks runs top-to-bottom in a strict linear pass. The file is split into two distinct execution zones by the engine:

```
┌────────────────────────────────────────────────────────┐
│                      EFFECT ZONE                       │
│  (Statements, Handlers, DB Mutations, Cookie Sessions) │
│                                                        │
│  • May invoke side effects: db.exec, redirect, cookies │
│  • Runs BEFORE the first byte of visible HTML          │
│  • If redirect() is called: HALT execution immediately  │
└───────────────────────────┬────────────────────────────┘
                            │ (First HTML output byte)
┌───────────────────────────▼────────────────────────────┐
│                       VIEW ZONE                        │
│                 (HTML & Read-Only Stream)              │
│                                                        │
│  • Headers & cookies already committed                 │
│  • Output is streamed in chunks                        │
│  • Side effects (db.exec, redirect) are FORBIDDEN       │
└────────────────────────────────────────────────────────┘
```

---

## 2. Block-by-Block Annotation

### Block 1: File-Level Variable Declarations

```html
|| String $current_user = null;
   Json $tasks = [];
|
```

* **What it does:** Declares file-scoped variables `$current_user` and `$tasks` with explicit baseline types.
* **Ties to Sticks Philosophy:**
  * **Declared Types (§5.2):** Variables must be statically or explicitly declared before use. Universal `null` is legal for any type.
  * **Predictable Initialization (§5.4):** Top-to-bottom initialization ensures that when later handler or view blocks inspect `$current_user`, the identifier is guaranteed to exist.

---

### Block 2: Effect Zone Handler (`on POST`)

```html
|| on POST:
     if $req.body.action == "login" {
         session.set("user", $req.body.user);
         redirect("/");
     }
     if $req.body.action == "logout" {
         session.clear();
         redirect("/");
     }
     if $req.body.action == "add" {
         String $u = session.get("user");
         db.exec("INSERT INTO tasks (username, name, done) VALUES (?, ?, 0)", [$u, $req.body.task_name]);
         redirect("/");
     }
     if $req.body.action == "toggle" {
         db.exec("UPDATE tasks SET done = 1 - done WHERE id = ?", [$req.body.task_id]);
         redirect("/");
     }
     if $req.body.action == "delete" {
         db.exec("DELETE FROM tasks WHERE id = ?", [$req.body.task_id]);
         redirect("/");
     }
 |
```

* **What it does:** Intercepts incoming HTTP `POST` requests and processes state mutations (logging in, logging out, inserting tasks, toggling completion, deleting tasks). Once an action completes, it triggers `redirect("/")`.
* **Ties to Sticks Philosophy:**
  * **Effects Isolation (§5.6, §6):** Mutations (`db.exec`, `session.set`, `redirect`) are **strictly prohibited** in the View Zone. They are only legal inside an `on` handler.
  * **Literal SQL Constraints (§8.1):** Every query string is an immutable literal. Dynamic query concatenation is rejected by the compiler to structurally prevent SQL injection.
  * **Post-Redirect-Get (PRG):** Invoking `redirect("/")` halts execution immediately. No view code is evaluated on a mutation request; the browser is told to fetch a fresh view via `GET`.

---

### Block 3: State Population Handler (`on GET`)

```html
|| on GET:
     $current_user = session.get("user");
     if $req.query.user != null {
         $current_user = $req.query.user;
     }
 |
```

* **What it does:** Runs on incoming HTTP `GET` requests. It first reads the persistent session cookie via `session.get("user")`. If a URL query parameter (`?user=...`) is provided, it allows an override.
* **Ties to Sticks Philosophy:**
  * **Separation of Concerns (§6):** Reads do not alter database state. They only prepare data for the view.
  * **Fallback Resolution:** Allows smooth interoperability between session-based cookies and query-string navigation.

---

### Block 4: Read-Only Data Fetching

```html
|| Result $rows = null;
   if $current_user != null {
     $rows = db.all("SELECT id, username, name, done FROM tasks WHERE username = ? ORDER BY id ASC", [$current_user]);
     if $rows.ok == true {
         $tasks = $rows.value;
     }
   }
|
```

* **What it does:** If an authenticated user is present, it issues a read query (`db.all`) to fetch their tasks. If the query succeeds (`$rows.ok == true`), `$tasks` is assigned the resulting array.
* **Ties to Sticks Philosophy:**
  * **Result Object Contract (§8.1, §10):** I/O calls do not throw unhandled runtime exceptions. They return a structured `Result` containing `ok` and `value`.
  * **End of Effect Zone:** This is the final code block before the markup begins. As soon as the scanner reaches the next byte (`<!DOCTYPE html>`), the transaction commits and the View Zone begins streaming.

---

### Block 5: The Auth Boundary (View Zone Header)

```html
|| if $current_user != null:
    <h1>Tasks for $current_user</h1>
    <form method="POST" action="/" style="display:inline;">
        <input type="hidden" name="action" value="logout">
        <button type="submit">Log Out / Switch User</button>
    </form>
    <hr>
    <ul>
||| else:
    <h1>Login to Tasks</h1>
    <form method="POST" action="/">
        <input type="hidden" name="action" value="login">
        <input type="text" name="user" placeholder="Enter username..." required autofocus>
        <button type="submit">View Tasks</button>
    </form>
|
```

* **What it does:** Text-mode conditional block. If `$current_user` is present, it renders the dashboard header and opens the `<ul>`. Otherwise, it renders the login form.
* **Ties to Sticks Philosophy:**
  * **Text-Mode Body (§5.1):** The block starts with `if ` and uses `:` to delimit the condition from the HTML body.
  * **Native Interpolation (§5.5):** Inside text-mode bodies, variables like `$current_user` are interpolated automatically. No nested `|| $current_user |` syntax is required.
  * **Strict Chaining Rule S3:** The `|||` token is simply the closing stick `|` followed by the opening sticks `||` without intervening whitespace.

---

### Block 6: Iteration Over Tasks

```html
|| for Json $t in $tasks:
    <li class="task status-$t.done">
        <span class="task-text" style="flex-grow: 1;">$t.name</span>
        
        <form method="POST" action="/" style="display:inline;">
            <input type="hidden" name="action" value="toggle">
            <input type="hidden" name="task_id" value="$t.id">
            <button type="submit">Toggle</button>
        </form>

        <form method="POST" action="/" style="display:inline;">
            <input type="hidden" name="action" value="delete">
            <input type="hidden" name="task_id" value="$t.id">
            <button type="submit" style="color: red;">X</button>
        </form>
    </li>
|
```

* **What it does:** Loops through the `$tasks` array and emits an `<li>` for each task containing toggle and delete forms.
* **Ties to Sticks Philosophy:**
  * **Type-Bound Iteration:** `for Json $t in $tasks:` explicitly binds the iteration variable.
  * **Safe Output Encoding (§5.7):** Property accesses (`$t.name`, `$t.id`) are HTML-escaped automatically by type to prevent XSS.

---

### Block 7: The Closing Boundary (View Zone Footer)

```html
|| if $current_user != null:
    </ul>
    <hr>
    <form method="POST" action="/" style="display: flex; gap: 0.5rem;">
        <input type="hidden" name="action" value="add">
        <input type="text" name="task_name" placeholder="New task..." required>
        <button type="submit">Add</button>
    </form>
|
```

* **What it does:** If the user is logged in, closes the `</ul>` opened in Block 5 and renders the form to create new tasks.
* **Ties to Sticks Philosophy:**
  * Mirrors Block 5 to maintain clean, well-formed HTML without wrapping the loop in an outer conditional block.

---

## 3. How the "No Nesting" Rule (Rule S4) Was Solved

Rule S4 states:
> **A block cannot start inside a block. Block bodies are text or code, never more blocks.**

In traditional templating engines (Jinja, Blade, EJS), this application would typically use nested blocks:
1. An outer `if $logged_in` wrapping the whole list and the loop.
2. An inner `if $task.done` inside the `for` loop to strike through completed tasks.

Both patterns are illegal in Sticks. Here is how both were solved cleanly:

### Problem 1: Structural Wrapping (Outer `if` around a `for` loop)

#### The Illegal Approach
```html
<!-- ILLEGAL: Nesting a `for` block inside an `if` block -->
|| if $current_user != null:
    <ul>
      || for Json $t in $tasks:
          <li>$t.name</li>
      |
    </ul>
|
```

#### The Sticks Solution: The Sibling Block Pattern
Instead of nesting, the template splits the wrapper into sequential **sibling blocks**:

1. **Sibling Block A (Header):** If authenticated, emit `<h1>` and the opening `<ul>`. If unauthenticated, emit the login form.
2. **Sibling Block B (The Loop):** Sits at the root level between the two conditions. If unauthenticated, `$tasks` is empty, so the loop naturally iterates zero times and emits nothing.
3. **Sibling Block C (Footer):** If authenticated, emit the closing `</ul>` and the task creation form.

```
[Block 5: If logged in -> <ul>]
             │
[Block 6: Root-level Loop over $tasks]
             │
[Block 7: If logged in -> </ul>]
```

---

### Problem 2: Branching UI Logic Inside a Loop

#### The Illegal Approach
```html
<!-- ILLEGAL: Nesting an `if` block inside a `for` text-mode body -->
|| for Json $t in $tasks:
    <li>
      || if $t.done == 1:
          <s>$t.name</s>
      ||| else:
          <span>$t.name</span>
      |
    </li>
|
```

#### The Sticks Solution: CSS State Projection
Instead of branching markup on the server, the server projects the database field directly into the HTML class attribute, leaving the conditional visual rendering to CSS:

1. **Markup Projection:**
   ```html
   <li class="task status-$t.done">
       <span class="task-text">$t.name</span>
       ...
   </li>
   ```
2. **CSS Presentation Rule:**
   ```css
   .status-1 .task-text { 
       text-decoration: line-through; 
       color: gray; 
   }
   ```

* **Result:** No nested template tags are needed. The scanner stays a simple, single-pass two-state machine, while the browser handles visual branching natively.
