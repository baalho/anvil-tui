//! Guided project templates for kids mode.
//!
//! Projects are step-by-step guided experiences with prompts, hints,
//! and verification commands. They make kids mode useful by providing
//! structure instead of a blank terminal prompt.
//!
//! Each project is a TOML definition with steps. The `/project` command
//! manages the lifecycle: list, start, next, hint.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A guided project definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub project: ProjectMeta,
    pub steps: Vec<ProjectStep>,
}

/// Project metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub description: String,
    pub difficulty: String,
    pub estimated_minutes: u32,
}

/// A single step in a guided project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStep {
    /// The prompt to show the user (what to ask the agent).
    pub prompt: String,
    /// A hint the user can request if stuck.
    pub hint: String,
    /// Shell command to verify the step was completed.
    pub verify: String,
}

/// Active project state — tracks which project is running and current step.
#[derive(Debug, Clone)]
pub struct ActiveProject {
    pub project: Project,
    pub current_step: usize,
    pub workspace: PathBuf,
}

impl ActiveProject {
    /// Get the current step, or None if all steps are complete.
    pub fn current(&self) -> Option<&ProjectStep> {
        self.project.steps.get(self.current_step)
    }

    /// Advance to the next step. Returns the new step or None if done.
    pub fn advance(&mut self) -> Option<&ProjectStep> {
        self.current_step += 1;
        self.current()
    }

    /// Total number of steps.
    pub fn total_steps(&self) -> usize {
        self.project.steps.len()
    }

    /// Whether all steps are complete.
    pub fn is_complete(&self) -> bool {
        self.current_step >= self.project.steps.len()
    }

    /// Run the verification command for the current step.
    pub fn verify_current(&self) -> Result<bool> {
        let step = match self.current() {
            Some(s) => s,
            None => return Ok(true),
        };

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&step.verify)
            .current_dir(&self.workspace)
            .output()?;

        Ok(output.status.success())
    }

    /// Format the current step as a display string.
    pub fn format_current_step(&self) -> String {
        match self.current() {
            Some(step) => {
                format!(
                    "📋 Step {}/{}: {}\n\n  💡 Say this to your coding buddy:\n  \"{}\"",
                    self.current_step + 1,
                    self.total_steps(),
                    self.project.project.name,
                    step.prompt
                )
            }
            None => format!("🎉 You finished {}! Great job!", self.project.project.name),
        }
    }
}

/// Load all bundled project templates.
pub fn bundled_projects() -> Vec<Project> {
    vec![
    // hello-web: HTML/CSS webpage
    Project {
        project: ProjectMeta {
            name: "My First Website".to_string(),
            description: "Build a simple webpage with HTML and CSS".to_string(),
            difficulty: "beginner".to_string(),
            estimated_minutes: 10,
        },
        steps: vec![
            ProjectStep {
                prompt: "Create a file called index.html with a heading that says Hello World"
                    .to_string(),
                hint: "Use the <h1> tag for headings".to_string(),
                verify: "test -f index.html && grep -qi 'hello' index.html".to_string(),
            },
            ProjectStep {
                prompt: "Add a paragraph below the heading about your favorite animal".to_string(),
                hint: "Use the <p> tag for paragraphs".to_string(),
                verify: "grep -q '<p>' index.html".to_string(),
            },
            ProjectStep {
                prompt: "Add some CSS to make the heading blue and the background yellow"
                    .to_string(),
                hint: "You can use a <style> tag in the <head> section".to_string(),
                verify: "grep -qi 'color' index.html || grep -qi 'style' index.html".to_string(),
            },
            ProjectStep {
                prompt: "Add an image of a star using an emoji or HTML entity".to_string(),
                hint: "Try ⭐ or &star; in your HTML".to_string(),
                verify: "grep -qE '⭐|star|img' index.html".to_string(),
            },
        ],
    },

    // number-game: Python number guessing game
    Project {
        project: ProjectMeta {
            name: "Number Guessing Game".to_string(),
            description: "Build a Python game where you guess a secret number".to_string(),
            difficulty: "beginner".to_string(),
            estimated_minutes: 15,
        },
        steps: vec![
            ProjectStep {
                prompt: "Create a Python file called game.py that picks a random number between 1 and 10".to_string(),
                hint: "Use import random and random.randint(1, 10)".to_string(),
                verify: "test -f game.py && grep -q 'random' game.py".to_string(),
            },
            ProjectStep {
                prompt: "Add code that asks the player to guess the number using input()".to_string(),
                hint: "Use input('Guess a number: ') to ask the player".to_string(),
                verify: "grep -q 'input' game.py".to_string(),
            },
            ProjectStep {
                prompt: "Add code that tells the player if they guessed too high, too low, or got it right".to_string(),
                hint: "Use if/elif/else to compare the guess with the secret number".to_string(),
                verify: "grep -qE 'too high|too low|correct|right|win' game.py".to_string(),
            },
            ProjectStep {
                prompt: "Add a loop so the player can keep guessing until they get it right".to_string(),
                hint: "Use a while loop that breaks when the guess is correct".to_string(),
                verify: "grep -q 'while' game.py".to_string(),
            },
        ],
    },

    // story-bot: Interactive story generator
    Project {
        project: ProjectMeta {
            name: "Story Bot".to_string(),
            description: "Create an interactive story where you make choices".to_string(),
            difficulty: "intermediate".to_string(),
            estimated_minutes: 20,
        },
        steps: vec![
            ProjectStep {
                prompt: "Create a file called story.py that prints a welcome message and the start of an adventure story".to_string(),
                hint: "Use print() to show the story text".to_string(),
                verify: "test -f story.py && grep -q 'print' story.py".to_string(),
            },
            ProjectStep {
                prompt: "Add a choice where the player picks between two paths (like go left or go right)".to_string(),
                hint: "Use input() to ask and if/else to handle the choice".to_string(),
                verify: "grep -q 'input' story.py && grep -qE 'if|choice' story.py".to_string(),
            },
            ProjectStep {
                prompt: "Add a second choice on one of the paths with a different adventure".to_string(),
                hint: "Nest another input() and if/else inside the first choice".to_string(),
                verify: "python3 -c \"import ast; ast.parse(open('story.py').read())\"".to_string(),
            },
            ProjectStep {
                prompt: "Add a happy ending and a scary ending depending on the choices".to_string(),
                hint: "Each path should end with a different print() message".to_string(),
                verify: "grep -cE 'print' story.py | grep -qE '[3-9]|[0-9]{2}'".to_string(),
            },
        ],
    },
    ]
}

/// Find a project by name (case-insensitive, supports partial match).
pub fn find_project(name: &str) -> Option<Project> {
    let lower = name.to_lowercase();
    let projects = bundled_projects();

    // Exact match first
    if let Some(p) = projects
        .iter()
        .find(|p| p.project.name.to_lowercase() == lower)
    {
        return Some(p.clone());
    }

    // Partial match on name or description
    projects.into_iter().find(|p| {
        p.project.name.to_lowercase().contains(&lower)
            || p.project.description.to_lowercase().contains(&lower)
    })
}

/// Format the project list for display.
pub fn format_project_list() -> String {
    let projects = bundled_projects();
    if projects.is_empty() {
        return "no projects available".to_string();
    }

    let mut out = String::from("Available projects:\n\n");
    for p in &projects {
        out.push_str(&format!(
            "  📦 {} — {} ({}, ~{} min)\n",
            p.project.name,
            p.project.description,
            p.project.difficulty,
            p.project.estimated_minutes
        ));
    }
    out.push_str("\nStart one with: /project start <name>");
    out
}

/// Start a project — creates workspace dir and returns the active project.
pub fn start_project(name: &str, workspace: &Path) -> Result<ActiveProject> {
    let project = match find_project(name) {
        Some(p) => p,
        None => bail!("no project matching '{}'. Try /project list", name),
    };

    // Create a project subdirectory
    let project_dir = workspace.join(project.project.name.to_lowercase().replace(' ', "-"));
    std::fs::create_dir_all(&project_dir)?;

    Ok(ActiveProject {
        project,
        current_step: 0,
        workspace: project_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn bundled_projects_exist() {
        let projects = bundled_projects();
        assert_eq!(projects.len(), 3);
        assert_eq!(projects[0].project.name, "My First Website");
        assert_eq!(projects[1].project.name, "Number Guessing Game");
        assert_eq!(projects[2].project.name, "Story Bot");
    }

    #[test]
    fn find_project_by_name() {
        assert!(find_project("website").is_some());
        assert!(find_project("number").is_some());
        assert!(find_project("story").is_some());
        assert!(find_project("nonexistent").is_none());
    }

    #[test]
    fn project_lifecycle() {
        let dir = TempDir::new().unwrap();
        let active = start_project("website", dir.path()).unwrap();

        assert_eq!(active.current_step, 0);
        assert_eq!(active.total_steps(), 4);
        assert!(!active.is_complete());
        assert!(active.current().is_some());
        assert!(active.workspace.exists());
    }

    #[test]
    fn project_step_advancement() {
        let dir = TempDir::new().unwrap();
        let mut active = start_project("website", dir.path()).unwrap();

        assert_eq!(active.current_step, 0);
        active.advance();
        assert_eq!(active.current_step, 1);
        active.advance();
        assert_eq!(active.current_step, 2);
        active.advance();
        assert_eq!(active.current_step, 3);
        active.advance();
        assert!(active.is_complete());
    }

    #[test]
    fn format_project_list_shows_all() {
        let list = format_project_list();
        assert!(list.contains("My First Website"));
        assert!(list.contains("Number Guessing Game"));
        assert!(list.contains("Story Bot"));
        assert!(list.contains("/project start"));
    }
}
