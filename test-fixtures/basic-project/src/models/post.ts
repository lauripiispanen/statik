import { User } from "./user";

export interface Post {
  id: string;
  title: string;
  content: string;
  author: User;
  publishedAt: Date;
  tags: string[];
}

export interface CreatePostInput {
  title: string;
  content: string;
  authorId: string;
  tags?: string[];
}

export interface PostComment {
  id: string;
  postId: string;
  author: User;
  text: string;
  createdAt: Date;
}

export function slugify(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "");
}

export function createPost(input: CreatePostInput, author: User): Post {
  return {
    id: Math.random().toString(36).substring(7),
    title: input.title,
    content: input.content,
    author,
    publishedAt: new Date(),
    tags: input.tags || [],
  };
}
